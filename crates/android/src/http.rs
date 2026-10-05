//! Serial HTTP bridge on `127.0.0.1`: every verb except `launch` and
//! `terminate` relays byte-for-byte to the forwarded on-device listener;
//! the two lifecycle verbs run locally against `adb` and call the driver
//! back through [`Wire`] for their snapshot/status halves.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;

use crate::driver::SecretToken;
use crate::lifecycle::LifecycleControl;
use crate::proxy::{IO_TIMEOUT, error_envelope, http_response, read_request};
use crate::routes::lifecycle_route;

mod relay;
use relay::relay;

/// A running bridge: one accept loop feeding at most one worker at a
/// time — complete requests that arrive while a command is in flight get
/// a 503, not a backlog slot — bounded and idempotent [`Bridge::stop`].
pub(crate) struct Bridge {
    port: u16,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Option<TcpStream>>>,
    #[cfg(test)]
    accepted: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>>,
    handle: Option<JoinHandle<()>>,
}

/// Bind `127.0.0.1:0` and start the serial accept loop.
///
/// # Errors
/// [`Failure::Local`] on bind/thread failures.
pub(crate) fn start_bridge(
    upstream_port: u16,
    token: &SecretToken,
    lifecycle: Arc<dyn LifecycleControl>,
) -> Result<Bridge, Failure> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(Failure::from)?;
    start_on(
        listener,
        upstream_port,
        token.clone(),
        lifecycle,
        REQUEST_DEADLINE,
    )
}

/// Is this `accept` failure worth another iteration? Only `Interrupted`
/// is transient; permanent errors (listener closed) break the loop.
fn accept_retryable(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::Interrupted
}

/// Absolute deadline for reading one client request — independent of the
/// per-read idle timeout, so a byte trickle cannot hold a socket open.
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

/// Wraps a socket so every `read` carries the lesser of the per-read idle
/// budget and the time left on the absolute request deadline.
struct DeadlineReader<'a> {
    sock: &'a mut TcpStream,
    deadline: Instant,
}

impl DeadlineReader<'_> {
    fn new(sock: &mut TcpStream, deadline: Duration) -> DeadlineReader<'_> {
        DeadlineReader {
            sock,
            deadline: Instant::now() + deadline,
        }
    }
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "request deadline exhausted",
            ));
        }
        self.sock
            .set_read_timeout(Some(remaining.min(IO_TIMEOUT)))?;
        self.sock.read(buf)
    }
}

/// `std::thread::spawn` named so failure paths stay [`Failure`]-shaped.
fn thread_spawn(f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>, Failure> {
    std::thread::Builder::new().spawn(f).map_err(Failure::from)
}

/// Loop-owned values bundled so `serve` stays under the arg lint.
struct ServeArgs {
    listener: TcpListener,
    upstream_port: u16,
    token: SecretToken,
    lifecycle: Arc<dyn LifecycleControl>,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Option<TcpStream>>>,
    accepted: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>>,
    request_deadline: Duration,
}

fn start_on(
    listener: TcpListener,
    upstream_port: u16,
    token: SecretToken,
    lifecycle: Arc<dyn LifecycleControl>,
    request_deadline: Duration,
) -> Result<Bridge, Failure> {
    let port = listener.local_addr().map_err(Failure::from)?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let active: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
    let accepted = Arc::new(Mutex::new(None));
    let handle = {
        let stop = stop.clone();
        let active = active.clone();
        let accepted = accepted.clone();
        thread_spawn(move || {
            serve(ServeArgs {
                listener,
                upstream_port,
                token,
                lifecycle,
                stop,
                active,
                accepted,
                request_deadline,
            });
        })?
    };
    Ok(Bridge {
        port,
        stop,
        active,
        #[cfg(test)]
        accepted,
        handle: Some(handle),
    })
}

/// [`start_bridge`] with an explicit request deadline — tests shrink it
/// to prove the absolute budget independently of the idle timeout.
#[cfg(test)]
fn start_bridge_with_deadline(
    upstream_port: u16,
    token: &SecretToken,
    lifecycle: Arc<dyn LifecycleControl>,
    request_deadline: Duration,
) -> Result<Bridge, Failure> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(Failure::from)?;
    start_on(
        listener,
        upstream_port,
        token.clone(),
        lifecycle,
        request_deadline,
    )
}

/// Accept loop: serial, one action at a time, exits on `stop`.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the loop body owns these values inside a 'static thread"
)]
fn serve(args: ServeArgs) {
    let ServeArgs {
        listener,
        upstream_port,
        token,
        lifecycle,
        stop,
        active,
        accepted,
        request_deadline,
    } = args;
    let mut worker: Option<JoinHandle<()>> = None;
    while !stop.load(Ordering::SeqCst) {
        let (mut sock, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if accept_retryable(&e) => continue,
            Err(_) => break,
        };
        if stop.load(Ordering::SeqCst) {
            let _ = sock.shutdown(Shutdown::Both);
            break;
        }
        if worker.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(done) = worker.take()
        {
            let _ = done.join();
        }
        if worker.is_some() {
            let _ = sock.set_write_timeout(Some(IO_TIMEOUT));
            let body = error_envelope(None, None, "DRIVER_ERROR", "another command is in progress");
            let _ = sock.write_all(&http_response("503 Service Unavailable", &body));
            let _ = sock.shutdown(Shutdown::Both);
            continue;
        }
        {
            let Ok(mut slot) = active.lock() else {
                break;
            };
            *slot = sock.try_clone().ok();
        }
        if let Ok(guard) = accepted.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(());
        }
        let Ok(mut sock_w) = sock.try_clone() else {
            if let Ok(mut slot) = active.lock() {
                *slot = None;
            }
            let _ = sock.set_write_timeout(Some(IO_TIMEOUT));
            let body = error_envelope(
                None,
                None,
                "DRIVER_ERROR",
                "client socket could not be cloned",
            );
            let _ = sock.write_all(&http_response("500 Internal Server Error", &body));
            continue;
        };
        let active_w = active.clone();
        let (token_w, lifecycle_w) = (token.clone(), lifecycle.clone());
        worker = if let Ok(h) = thread_spawn(move || {
            handle_client(
                &mut sock_w,
                upstream_port,
                &token_w,
                &lifecycle_w,
                request_deadline,
            );
            if let Ok(mut slot) = active_w.lock() {
                *slot = None;
            }
        }) {
            Some(h)
        } else {
            if let Ok(mut slot) = active.lock() {
                *slot = None;
            }
            let _ = sock.set_write_timeout(Some(IO_TIMEOUT));
            let body = error_envelope(None, None, "DRIVER_ERROR", "driver worker unavailable");
            let _ = sock.write_all(&http_response("500 Internal Server Error", &body));
            None
        };
    }
    if let Some(h) = worker.take() {
        let _ = h.join();
    }
}

/// Read and dispatch one request; every failure path still writes an
/// envelope-shaped reply where a request was readable.
fn handle_client(
    sock: &mut TcpStream,
    upstream_port: u16,
    token: &SecretToken,
    lifecycle: &Arc<dyn LifecycleControl>,
    request_deadline: Duration,
) {
    let started = Instant::now();
    let _ = sock.set_write_timeout(Some(IO_TIMEOUT));
    let Ok(req) = read_request(&mut DeadlineReader::new(sock, request_deadline)) else {
        return;
    };
    let route = req.path.split('?').next().unwrap_or("");
    match route {
        "/launch" | "/terminate" => {
            lifecycle_route(
                sock,
                &req,
                &route[1..],
                upstream_port,
                token,
                lifecycle,
                started,
            );
        }
        _ => relay(sock, &req, upstream_port, started),
    }
}
impl Bridge {
    /// Port the bridge bound on `127.0.0.1`.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Whether the accept thread is alive and no stop was requested.
    pub(crate) fn is_running(&self) -> bool {
        !self.stop.load(Ordering::SeqCst) && self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Test hook: signal each accepted client after it is installed in
    /// `active`, so shutdown tests can synchronize without sleeps.
    #[cfg(test)]
    pub(crate) fn notify_on_accept(&self, tx: std::sync::mpsc::Sender<()>) {
        if let Ok(mut guard) = self.accepted.lock() {
            *guard = Some(tx);
        }
    }

    /// Bounded idempotent shutdown: flag the loop, close the in-flight
    /// client, wake the blocked accept, and join the thread.
    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.active.lock()
            && let Some(client) = slot.take()
        {
            let _ = client.shutdown(Shutdown::Both);
        }
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
use relay::read_reply;

#[cfg(test)]
mod tests;
