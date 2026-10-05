package com.lahfir.agentmobile.driver

import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest
import org.json.JSONObject

internal class HttpServer(
    private val port: Int = 8770,
    private val token: String,
    private val handler: (String, JSONObject) -> JSONObject,
    private val socketTimeoutMs: Int = 10_000,
    private val maxHeaderBytes: Int = 1_048_576,
    private val maxBodyBytes: Int = 16_777_216,
) : AutoCloseable {

    private val lock = Any()
    private var serverSocket: ServerSocket? = null
    private var acceptThread: Thread? = null
    private var activeClient: Socket? = null
    private var closed = false

    val localPort: Int
        get() = serverSocket?.localPort ?: -1

    internal fun hasActiveClient(): Boolean = synchronized(lock) { activeClient != null }

    fun start() {
        check(serverSocket == null) { "server already started" }
        val socket = ServerSocket()
        socket.reuseAddress = true
        socket.bind(InetSocketAddress(InetAddress.getByName(BIND_ADDRESS), port), BACKLOG)
        serverSocket = socket
        acceptThread = Thread({ acceptLoop(socket) }, THREAD_NAME).apply {
            isDaemon = true
            start()
        }
    }

    override fun close() {
        val client: Socket?
        synchronized(lock) {
            closed = true
            client = activeClient
        }
        serverSocket?.let {
            try {
                it.close()
            } catch (_: IOException) {
            }
        }
        if (client != null) {
            try {
                client.close()
            } catch (_: IOException) {
            }
        }
        acceptThread?.join(socketTimeoutMs.toLong() + JOIN_MARGIN_MS)
        synchronized(lock) {
            serverSocket = null
            activeClient = null
        }
        acceptThread = null
    }

    private fun acceptLoop(socket: ServerSocket) {
        while (!socket.isClosed) {
            val client = try {
                socket.accept()
            } catch (_: IOException) {
                break
            }
            val refuse = synchronized(lock) {
                if (closed) true else {
                    activeClient = client
                    false
                }
            }
            if (refuse) {
                try {
                    client.close()
                } catch (_: IOException) {
                }
                break
            }
            try {
                serve(client)
            } catch (_: Exception) {
            } finally {
                synchronized(lock) { if (activeClient === client) activeClient = null }
                try {
                    client.close()
                } catch (_: IOException) {
                }
            }
        }
    }

    private fun serve(client: Socket) {
        client.soTimeout = socketTimeoutMs
        val startMs = System.currentTimeMillis()
        val input = BufferedInputStream(client.getInputStream())

        val headBytes = readHead(input) ?: return
        val head = String(headBytes, Charsets.ISO_8859_1)
        val lines = head.split("\r\n")
        val requestParts = lines[0].split(" ")
        if (requestParts.size < 2) return
        val method = requestParts[0]
        val path = requestParts[1]
        val headers = HashMap<String, String>()
        for (i in 1 until lines.size) {
            val line = lines[i]
            if (line.isEmpty()) continue
            val sep = line.indexOf(':')
            if (sep < 0) continue
            headers[line.substring(0, sep).trim().lowercase()] = line.substring(sep + 1).trim()
        }

        val command = path.substringBefore('?').trim('/')
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
        if (method != "POST") {
            writeResponse(client, 405, failure(command, startMs, "BAD_REQUEST", "verbs are POST only"), headers)
            return
        }
        if (headers["x-agent-mobile-version"] != Protocol.VERSION) {
            writeResponse(client, 409, failure(command, startMs, "BAD_REQUEST", "X-Agent-Mobile-Version must be 1"), headers)
            return
        }

        val contentLengthHeader = headers["content-length"]
        val contentLength = if (contentLengthHeader != null) {
            contentLengthHeader.toIntOrNull() ?: -1
        } else {
            0
        }
        if (contentLength < 0 || contentLength > maxBodyBytes) {
            writeResponse(client, 409, failure(command, startMs, "BAD_REQUEST", "invalid Content-Length"), headers)
            return
        }

        val body = ByteArray(contentLength)
        var offset = 0
        while (offset < contentLength) {
            val n = try {
                input.read(body, offset, contentLength - offset)
            } catch (_: IOException) {
                return
            }
            if (n < 0) return
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

        val (status, envelope) = try {
            200 to Protocol.success(command, elapsedSince(startMs), handler(command, params))
        } catch (e: DriverException) {
            409 to failure(command, startMs, e.code, e.message ?: e.code)
        } catch (e: Exception) {
            500 to failure(command, startMs, "DRIVER_ERROR", e.message ?: e.javaClass.simpleName)
        }
        writeResponse(client, status, envelope, headers)
    }

    private fun readHead(input: InputStream): ByteArray? {
        val buf = ByteArrayOutputStream()
        var window = 0
        while (true) {
            if (buf.size() >= maxHeaderBytes) return null
            val b = try {
                input.read()
            } catch (_: IOException) {
                return null
            }
            if (b < 0) return null
            buf.write(b)
            window = (window shl 8) or b
            if (window == HEADER_END_MARKER) break
        }
        return buf.toByteArray()
    }

    private fun failure(command: String, startMs: Long, code: String, message: String): JSONObject =
        Protocol.failure(command, elapsedSince(startMs), code, message)

    private fun elapsedSince(startMs: Long): Long = System.currentTimeMillis() - startMs

    private fun writeResponse(client: Socket, status: Int, envelope: JSONObject, requestHeaders: Map<String, String>) {
        val body: ByteArray
        var contentType = "application/json"
        val data = envelope.optJSONObject("data")
        val text = data?.opt("text")
        if (requestHeaders["accept"] == "text/plain" && text is String) {
            val header = "app=${data.opt("app") ?: ""} snapshot=@${data.opt("snapshot_id") ?: ""}" +
                " refs=${data.opt("ref_count") ?: 0} settled=${data.opt("settled") ?: ""}" +
                " reads=${data.opt("reads") ?: ""} elapsed_ms=${envelope.opt("elapsed_ms") ?: ""}\n"
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
        private const val BACKLOG = 8
        private const val THREAD_NAME = "agent-mobile-http"
        private const val JOIN_MARGIN_MS = 1_000L
        private const val HEADER_END_MARKER = 0x0D0A0D0A
        private const val AUTH_MESSAGE = "Authorization: Bearer <token> required"
    }
}
