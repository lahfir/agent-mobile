package com.lahfir.agentmobile.driver

import java.net.Socket
import org.json.JSONObject

internal fun awaitUninterruptibly(latch: java.util.concurrent.CountDownLatch) {
        var interrupted = false
        while (true) {
            try {
                latch.await()
                break
            } catch (_: InterruptedException) {
                interrupted = true
            }
        }
        if (interrupted) {
            Thread.currentThread().interrupt()
        }
    }

internal fun exchange(port: Int, raw: String): HttpTestResponse? {
        Socket("127.0.0.1", port).use { socket ->
            socket.soTimeout = 10_000
            socket.getOutputStream().apply {
                write(raw.toByteArray(Charsets.UTF_8))
                flush()
            }
            socket.shutdownOutput()
            return readHttpResponseForTest(socket.getInputStream())
        }
    }

internal fun post(port: Int, path: String, token: String? = "tok", version: String? = "1", body: String = ""): HttpTestResponse? {
        val sb = StringBuilder("POST $path HTTP/1.1\r\n")
        sb.append("Host: 127.0.0.1\r\n")
        if (token != null) sb.append("Authorization: Bearer $token\r\n")
        if (version != null) sb.append("X-Agent-Mobile-Version: $version\r\n")
        sb.append("Content-Length: ${body.toByteArray(Charsets.UTF_8).size}\r\n")
        sb.append("Connection: close\r\n\r\n")
        sb.append(body)
        return exchange(port, sb.toString())
    }

internal fun statusHandler(app: String = "com.launcher"): (String, JSONObject, RequestCancellation) -> JSONObject =
        { command, _, _ ->
            if (command != "status") {
                throw DriverException("UNKNOWN_COMMAND", "unknown command: $command")
            }
            JSONObject()
                .put("app", app)
                .put("snapshot_id", "")
                .put("device", "emu64a")
                .put("os", "16")
        }

internal fun server(
        token: String = "tok",
        handler: (String, JSONObject, RequestCancellation) -> JSONObject = statusHandler(),
        socketTimeoutMs: Int = 5_000,
        requestDeadlineMs: Int = 30_000,
        clockMs: () -> Long = { System.nanoTime() / 1_000_000 },
        maxHeaderBytes: Int = 1_048_576,
        maxBodyBytes: Int = 16_777_216,
    ): HttpServer = HttpServer(
        port = 0,
        token = token,
        handler = handler,
        socketTimeoutMs = socketTimeoutMs,
        requestDeadlineMs = requestDeadlineMs,
        clockMs = clockMs,
        maxHeaderBytes = maxHeaderBytes,
        maxBodyBytes = maxBodyBytes,
    )

