package com.lahfir.agentmobile.driver

import android.os.SystemClock
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import org.json.JSONObject

internal class HttpServer(
    private val port: Int = 0,
    private val token: String,
    private val handler: (String, JSONObject, RequestCancellation) -> JSONObject,
    private val socketTimeoutMs: Int = 10_000,
    private val requestDeadlineMs: Int = 30_000,
    private val clockMs: () -> Long = { SystemClock.elapsedRealtime() },
    private val maxHeaderBytes: Int = HEAD_CAP,
    private val maxBodyBytes: Int = BODY_CAP,
) : AutoCloseable {

    private class RequestWork(
        val client: Socket,
        val cancellation: RequestCancellation,
        val startMs: Long,
        val deadlineMs: Long,
        val busy: Boolean = false,
    ) {
        val finished = java.util.concurrent.atomic.AtomicBoolean(false)

        @Volatile
        var watchdog: Thread? = null
    }

    private val lock = Any()
    private val queue = ArrayBlockingQueue<RequestWork>(1)
    private var serverSocket: ServerSocket? = null
    private var acceptThread: Thread? = null
    private var requestThread: Thread? = null
    private var activeWork: RequestWork? = null
    private var processingWork: RequestWork? = null
    private var busyClient: Socket? = null
    private var busyCancellation: RequestCancellation? = null
    private val deadlineThreads = HashSet<Thread>()
    private var closed = false

    val localPort: Int
        get() = synchronized(lock) { serverSocket?.localPort ?: -1 }

    internal fun hasActiveClient(): Boolean = synchronized(lock) {
        activeWork != null || processingWork?.finished?.get() == false
    }

    internal fun workerAlive(): Boolean = synchronized(lock) {
        acceptThread?.isAlive == true || requestThread?.isAlive == true ||
            deadlineThreads.any { it.isAlive }
    }

    fun start() {
        synchronized(lock) {
            check(!closed) { "closed server cannot be restarted" }
            check(serverSocket == null) { "server already started" }
            val socket = ServerSocket()
            socket.reuseAddress = true
            try {
                socket.bind(InetSocketAddress(InetAddress.getByName(BIND_ADDRESS), port), BACKLOG)
            } catch (e: Exception) {
                try {
                    socket.close()
                } catch (_: IOException) {
                }
                throw e
            }
            val acceptor = Thread({ acceptLoop(socket) }, THREAD_NAME)
            acceptor.isDaemon = true
            val worker = Thread({ requestLoop() }, REQUEST_THREAD_NAME)
            worker.isDaemon = true
            try {
                serverSocket = socket
                acceptThread = acceptor
                requestThread = worker
                acceptor.start()
                worker.start()
            } catch (e: Exception) {
                try {
                    socket.close()
                } catch (_: IOException) {
                }
                acceptor.interrupt()
                worker.interrupt()
                serverSocket = null
                closed = true
                throw e
            }
        }
    }

    override fun close() {
        // Real monotonic time only — injected clockMs is a test seam that
        // may stand still and must not stretch the join budget.
        val deadlineNanos = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(
            socketTimeoutMs.toLong() + JOIN_MARGIN_MS,
        )
        val listener: ServerSocket?
        val clients: List<Socket>
        val threads: List<Thread>
        synchronized(lock) {
            closed = true
            val works = listOfNotNull(activeWork, processingWork).distinct()
            works.forEach { it.cancellation.cancel() }
            busyCancellation?.cancel()
            listener = serverSocket
            clients = works.map { it.client } + listOfNotNull(busyClient)
            threads = (listOfNotNull(acceptThread, requestThread) + deadlineThreads)
                .distinct()
                .filter { it !== Thread.currentThread() }
        }
        listener?.let {
            try {
                it.close()
            } catch (_: IOException) {
            }
        }
        clients.forEach {
            try {
                it.close()
            } catch (_: IOException) {
            }
        }
        threads.forEach { it.interrupt() }
        val wasInterrupted = Thread.interrupted()
        var interruptCaught = false
        for (t in threads) {
            var remaining = deadlineNanos - System.nanoTime()
            while (remaining > 0 && t.isAlive) {
                try {
                    TimeUnit.NANOSECONDS.timedJoin(t, remaining)
                } catch (_: InterruptedException) {
                    interruptCaught = true
                }
                remaining = deadlineNanos - System.nanoTime()
            }
        }
        if (wasInterrupted || interruptCaught) {
            Thread.currentThread().interrupt()
        }
        synchronized(lock) {
            if (acceptThread?.isAlive != true && requestThread?.isAlive != true &&
                deadlineThreads.none { it.isAlive }
            ) {
                serverSocket = null
                acceptThread = null
                requestThread = null
                activeWork = null
                processingWork = null
                busyClient = null
                busyCancellation = null
            }
        }
    }

    private fun acceptLoop(socket: ServerSocket) {
        while (!Thread.currentThread().isInterrupted) {
            val client = try {
                socket.accept()
            } catch (_: IOException) {
                break
            }
            var busy: RequestWork? = null
            synchronized(lock) {
                when {
                    closed -> {
                        try {
                            client.close()
                        } catch (_: IOException) {
                        }
                    }
                    activeWork != null -> {
                        val now = clockMs()
                        val work = RequestWork(
                            client,
                            RequestCancellation(),
                            now,
                            now + requestDeadlineMs,
                            busy = true,
                        )
                        busyClient = client
                        busyCancellation = work.cancellation
                        armDeadlineWatcher(work)
                        busy = work
                    }
                    else -> {
                        val cancellation = RequestCancellation()
                        val now = clockMs()
                        val work = RequestWork(client, cancellation, now, now + requestDeadlineMs)
                        activeWork = work
                        armDeadlineWatcher(work)
                        queue.offer(work)
                    }
                }
            }
            busy?.let { serveBusy(it) }
        }
        try {
            socket.close()
        } catch (_: IOException) {
        }
    }

    private fun requestLoop() {
        while (true) {
            if (synchronized(lock) { closed }) {
                break
            }
            val work = try {
                queue.take()
            } catch (_: InterruptedException) {
                // May be a stale deadline interrupt, not shutdown — recheck
                // closed at the loop head rather than dying here.
                continue
            }
            var proceed = true
            synchronized(lock) {
                if (closed) {
                    proceed = false
                } else {
                    processingWork = work
                }
            }
            if (!proceed) {
                finalizeCancelled(work)
                break
            }
            Thread.interrupted()
            try {
                serveWithinDeadline(work.client, work.cancellation, work.startMs, work.deadlineMs)
            } catch (_: Exception) {
            } finally {
                synchronized(lock) {
                    work.finished.set(true)
                    if (activeWork === work) {
                        activeWork = null
                    }
                }
                try {
                    work.client.close()
                } catch (_: IOException) {
                }
                retireWatcher(work)
                synchronized(lock) {
                    if (processingWork === work) {
                        processingWork = null
                    }
                    if (closed) {
                        return
                    }
                }
            }
        }
    }

    private fun finalizeCancelled(work: RequestWork) {
        work.cancellation.cancel()
        try {
            work.client.close()
        } catch (_: IOException) {
        }
        synchronized(lock) {
            work.finished.set(true)
            if (activeWork === work) {
                activeWork = null
            }
            if (processingWork === work) {
                processingWork = null
            }
        }
        retireWatcher(work)
    }

    private fun armDeadlineWatcher(work: RequestWork) {
        val watcher = Thread {
            try {
                while (!work.finished.get()) {
                    val remaining = work.deadlineMs - clockMs()
                    if (remaining <= 0) {
                        var expired = false
                        synchronized(lock) {
                            if (!work.finished.get()) {
                                expired = true
                                work.cancellation.cancel()
                                if (!work.busy && processingWork === work) {
                                    requestThread?.interrupt()
                                }
                            }
                        }
                        if (expired) {
                            try {
                                work.client.close()
                            } catch (_: IOException) {
                            }
                        }
                        break
                    }
                    try {
                        Thread.sleep(minOf(remaining, 100L))
                    } catch (_: InterruptedException) {
                        break
                    }
                }
            } finally {
                synchronized(lock) {
                    deadlineThreads.remove(Thread.currentThread())
                }
            }
        }
        watcher.isDaemon = true
        watcher.name = DEADLINE_THREAD_NAME
        deadlineThreads.add(watcher)
        work.watchdog = watcher
        watcher.start()
    }

    private fun retireWatcher(work: RequestWork) {
        val watcher = work.watchdog
        if (watcher != null && watcher !== Thread.currentThread()) {
            watcher.interrupt()
            val wasInterrupted = Thread.interrupted()
            var interruptCaught = false
            try {
                watcher.join(JOIN_MARGIN_MS)
            } catch (_: InterruptedException) {
                interruptCaught = true
                try {
                    watcher.join(JOIN_MARGIN_MS)
                } catch (_: InterruptedException) {
                }
            }
            if (wasInterrupted || interruptCaught) {
                Thread.currentThread().interrupt()
            }
        }
        synchronized(lock) {
            if (watcher != null && !watcher.isAlive) {
                deadlineThreads.remove(watcher)
            }
        }
    }

    private fun serveBusy(work: RequestWork) {
        try {
            serveBusyWithinDeadline(work.client, work.cancellation)
        } catch (_: Exception) {
        } finally {
            synchronized(lock) {
                work.finished.set(true)
            }
            try {
                work.client.close()
            } catch (_: IOException) {
            }
            synchronized(lock) {
                if (busyClient === work.client) {
                    busyClient = null
                    busyCancellation = null
                }
            }
            retireWatcher(work)
        }
    }

    private fun serveWithinDeadline(
        client: Socket,
        cancellation: RequestCancellation,
        startMs: Long,
        deadlineMs: Long,
    ) {
        var readTimeoutMs = -1
        fun applyReadTimeout(): Unit {
            val remaining = deadlineMs - clockMs()
            if (remaining <= 0) {
                throw java.net.SocketTimeoutException("request deadline exceeded")
            }
            val next = minOf(socketTimeoutMs.toLong(), remaining).coerceAtLeast(1).toInt()
            if (next != readTimeoutMs) {
                client.soTimeout = next
                readTimeoutMs = next
            }
        }
        applyReadTimeout()
        val input = BufferedInputStream(client.getInputStream())

        val headBytes = try {
            readHead(input, ::applyReadTimeout) ?: return
        } catch (_: HttpFramingException) {
            writeResponse(client, 409, Protocol.failure(null, null, "BAD_REQUEST", FRAMING_MESSAGE), emptyMap())
            return
        }
        val head = try {
            HttpRequestHead.parse(headBytes, maxBodyBytes)
        } catch (_: HttpFramingException) {
            writeResponse(client, 409, Protocol.failure(null, null, "BAD_REQUEST", FRAMING_MESSAGE), emptyMap())
            return
        }
        val headers = head.headers

        val command = head.path.substringBefore('?').trim('/')
        val authorization = headers["authorization"]
        val authorized = token.isNotEmpty() && authorization != null &&
            MessageDigest.isEqual(
                "Bearer $token".toByteArray(Charsets.UTF_8),
                authorization.toByteArray(Charsets.UTF_8),
            )

        if (!authorized) {
            writeResponse(client, 401, Protocol.failure(null, null, "UNAUTHORIZED", AUTH_MESSAGE), headers)
            return
        }
        if (head.method != "POST") {
            writeResponse(client, 405, failure(command, startMs, "BAD_REQUEST", "verbs are POST only"), headers)
            return
        }
        if (headers["x-agent-mobile-version"] != Protocol.VERSION) {
            writeResponse(client, 409, failure(command, startMs, "BAD_REQUEST", "X-Agent-Mobile-Version must be 1"), headers)
            return
        }

        val body = ByteArray(head.contentLength)
        var offset = 0
        while (offset < head.contentLength) {
            applyReadTimeout()
            val n = try {
                input.read(body, offset, head.contentLength - offset)
            } catch (_: IOException) {
                return
            }
            if (n < 0) {
                writeResponse(client, 409, Protocol.failure(null, null, "BAD_REQUEST", FRAMING_MESSAGE), headers)
                return
            }
            offset += n
        }

        val params = try {
            if (body.isEmpty()) JSONObject() else JSONObject(String(body, Charsets.UTF_8))
        } catch (_: Exception) {
            null
        }
        if (params == null) {
            writeResponse(client, 409, failure(command, startMs, "BAD_REQUEST", "request body must be a JSON object"), headers)
            return
        }

        cancellation.check()
        val outcome = try {
            val data = handler(command, params, cancellation)
            cancellation.check()
            200 to Protocol.success(command, elapsedSince(startMs), data)
        } catch (e: DriverBusyException) {
            if (cancellation.isCancelled) null
            else 503 to failure(command, startMs, e.code, e.message ?: e.code)
        } catch (e: DriverException) {
            if (cancellation.isCancelled) null
            else Protocol.statusForCode(e.code) to failure(command, startMs, e.code, e.message ?: e.code)
        } catch (e: Exception) {
            if (cancellation.isCancelled) null
            else 500 to failure(command, startMs, "DRIVER_ERROR", e.message ?: e.javaClass.simpleName)
        }
        if (outcome != null) {
            writeResponse(client, outcome.first, outcome.second, headers)
        }
    }

    private fun readHead(input: InputStream, applyReadTimeout: () -> Unit): ByteArray? {
        val buf = ByteArrayOutputStream()
        var window = 0
        while (true) {
            applyReadTimeout()
            val b = try {
                input.read()
            } catch (_: IOException) {
                return null
            }
            if (b < 0) return null
            buf.write(b)
            window = (window shl 8) or b
            if (window == HEADER_END_MARKER) break
            if (buf.size() >= maxHeaderBytes) {
                throw HttpFramingException("request head too large")
            }
        }
        return buf.toByteArray()
    }

    private fun serveBusyWithinDeadline(client: Socket, cancellation: RequestCancellation) {
        try {
            writeResponse(
                client,
                503,
                Protocol.failure(null, null, "DRIVER_ERROR", BUSY_MESSAGE),
                emptyMap(),
            )
            // Snapshot the already-arrived backlog once — draining is only
            // about avoiding a TCP reset eating the reply, never about
            // reading the rejected request, and must never wait for more
            // bytes (a flooding peer would monopolize the acceptor).
            val input = client.getInputStream()
            var quota = input.available().coerceAtMost(BUSY_DRAIN_CAP)
            val chunk = ByteArray(minOf(quota, 8192).coerceAtLeast(1))
            while (quota > 0) {
                cancellation.check()
                val n = input.read(chunk, 0, minOf(quota, chunk.size))
                if (n <= 0) break
                quota -= n
            }
        } catch (_: IOException) {
        } catch (_: DriverException) {
        }
    }

    private fun failure(command: String, startMs: Long, code: String, message: String): JSONObject =
        Protocol.failure(command, elapsedSince(startMs), code, message)

    private fun elapsedSince(startMs: Long): Long = clockMs() - startMs

    private fun writeResponse(client: Socket, status: Int, envelope: JSONObject, requestHeaders: Map<String, String>) {
        val body: ByteArray
        var contentType = "application/json"
        val data = envelope.optJSONObject("data")
        val text = data?.opt("text")
        if (requestHeaders["accept"] == "text/plain" && text is String) {
            val incomplete = if (data.has("complete") && !data.optBoolean("complete", true)) {
                " complete=false"
            } else {
                ""
            }
            val header = "app=${data.opt("app") ?: ""} snapshot=@${data.opt("snapshot_id") ?: ""}" +
                " refs=${data.opt("ref_count") ?: 0} settled=${data.opt("settled") ?: ""}" +
                " reads=${data.opt("reads") ?: ""} elapsed_ms=${envelope.opt("elapsed_ms") ?: ""}$incomplete\n"
            body = (header + text + "\n").toByteArray(Charsets.UTF_8)
            contentType = "text/plain"
        } else {
            body = envelope.toString().toByteArray(Charsets.UTF_8)
        }
        val head = "HTTP/1.1 $status ${if (status == 200) "OK" else "Error"}\r\n" +
            "Content-Type: $contentType\r\n" +
            "Content-Length: ${body.size}\r\n" +
            "Connection: close\r\n\r\n"
        val out = client.getOutputStream()
        out.write(head.toByteArray(Charsets.ISO_8859_1))
        out.write(body)
        out.flush()
    }

    companion object {
        private const val BIND_ADDRESS = "127.0.0.1"
        internal const val HEAD_CAP = 1_048_576
        internal const val BODY_CAP = 16_777_216
        internal const val BUSY_DRAIN_CAP = HEAD_CAP + BODY_CAP
        private const val BACKLOG = 8
        private const val THREAD_NAME = "agent-mobile-http"
        private const val REQUEST_THREAD_NAME = "agent-mobile-http-request"
        private const val DEADLINE_THREAD_NAME = "agent-mobile-http-deadline"
        private const val FRAMING_MESSAGE = "malformed request"
        private const val BUSY_MESSAGE = "another command is in progress"
        private const val JOIN_MARGIN_MS = 1_000L
        private const val HEADER_END_MARKER = 0x0D0A0D0A
        private const val AUTH_MESSAGE = "Authorization: Bearer <token> required"
    }
}
