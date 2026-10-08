//! Android side of agent-mobile (P2): an ADB adapter that discovers devices
//! and AVDs, prepares the on-device accessibility driver, owns the forwarded
//! session, and bridges `launch`/`terminate` to ADB while proxying every
//! other verb to the driver's HTTP listener.

mod adb;
mod boot;
mod device;
mod driver;
mod forward;
mod http;
mod lifecycle;
mod proxy;
mod routes;
mod session;

#[cfg(test)]
mod testkit;

pub use device::{AndroidDeviceKind, AndroidDeviceState, AndroidScan, AndroidTarget, BootedAvd};
pub use driver::LEGACY_DEVICE_PORT;
pub use forward::ForwardJournal;
pub use lifecycle::{LifecycleControl, LifecycleError};
pub use session::{AndroidAdapter, AndroidSession, AndroidSessionMeta};
