package com.lahfir.agentmobile.driver

import java.net.Socket
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HttpServerTest {

    private data class Response(val status: Int, val headers: Map<String, String>, val body: String)

    private fun exchange(port: Int, raw: String): Response? {
        Socket("127.0.0.1", port).use { socket ->
            socket.soTimeout = 10_000
            socket.getOutputStream().apply {
                write(raw.toByteArray(Charsets.UTF_8))
                flush()
            }
            val bytes = socket.getInputStream().readBytes()
            if (bytes.isEmpty()) return null
            val text = String(bytes, Charsets.UTF_8)
            val head = text.substringBefore("\r\n\r\n")
            val lines = head.split("\r\n")
            val status = lines[0].split(" ")[1].toInt()
            val headers = lines.drop(1)
                .filter { it.contains(":") }
                .associate { it.substringBefore(":").trim().lowercase() to it.substringAfter(":").trim() }
            return Response(status, headers, text.substringAfter("\r\n\r\n", ""))
        }
    }

    private fun post(port: Int, path: String, token: String? = "tok", version: String? = "1", body: String = ""): Response? {
        val sb = StringBuilder("POST $path HTTP/1.1\r\n")
        sb.append("Host: 127.0.0.1\r\n")
        if (token != null) sb.append("Authorization: Bearer $token\r\n")
        if (version != null) sb.append("X-Agent-Mobile-Version: $version\r\n")
        sb.append("Content-Length: ${body.toByteArray(Charsets.UTF_8).size}\r\n")
        sb.append("Connection: close\r\n\r\n")
        sb.append(body)
        return exchange(port, sb.toString())
    }

    private fun statusHandler(app: String = "com.launcher"): (String, JSONObject) -> JSONObject =
        { command, _ ->
            if (command != "status") {
                throw DriverException("UNKNOWN_COMMAND", "unknown command: $command")
            }
            JSONObject()
                .put("app", app)
                .put("snapshot_id", "")
                .put("device", "emu64a")
                .put("os", "16")
        }

    private fun server(
        token: String = "tok",
        handler: (String, JSONObject) -> JSONObject = statusHandler(),
        socketTimeoutMs: Int = 5_000,
        maxHeaderBytes: Int = 1_048_576,
        maxBodyBytes: Int = 16_777_216,
    ): HttpServer = HttpServer(
        port = 0,
        token = token,
        handler = handler,
        socketTimeoutMs = socketTimeoutMs,
        maxHeaderBytes = maxHeaderBytes,
        maxBodyBytes = maxBodyBytes,
    )

    @Test
    fun statusRequestReturnsProtocolSuccess() {
        server().use { srv ->
            srv.start()
            assertTrue(srv.localPort > 0)
            val resp = post(srv.localPort, "/status")
            assertNotNull(resp)
            resp!!
            assertEquals(200, resp.status)
            assertEquals("application/json", resp.headers["content-type"])
            val envelope = JSONObject(resp.body)
            assertEquals("1", envelope.getString("version"))
            assertTrue(envelope.getBoolean("ok"))
            assertEquals("status", envelope.getString("command"))
            assertTrue(envelope.has("elapsed_ms"))
            assertEquals("com.launcher", envelope.getJSONObject("data").getString("app"))
        }
    }

    @Test
    fun missingBearerReturns401WithoutCommandOrElapsed() {
        server().use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status", token = null)
            assertEquals(401, resp!!.status)
            val envelope = JSONObject(resp.body)
            assertFalse(envelope.getBoolean("ok"))
            assertFalse(envelope.has("command"))
            assertFalse(envelope.has("elapsed_ms"))
            assertEquals("UNAUTHORIZED", envelope.getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun wrongBearerReturns401() {
        server().use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status", token = "wrong-token")
            assertEquals(401, resp!!.status)
            assertEquals("UNAUTHORIZED", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun nonPostMethodReturns405() {
        server().use { srv ->
            srv.start()
            val resp = exchange(
                srv.localPort,
                "GET /status HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nContent-Length: 0\r\n\r\n",
            )
            assertEquals(405, resp!!.status)
            assertEquals("BAD_REQUEST", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun wrongProtocolVersionReturns409() {
        server().use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status", version = "2")
            assertEquals(409, resp!!.status)
            assertEquals("BAD_REQUEST", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun unknownCommandReturns409UnknownCommand() {
        server().use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/explode")
            assertEquals(409, resp!!.status)
            val envelope = JSONObject(resp.body)
            assertEquals("UNKNOWN_COMMAND", envelope.getJSONObject("error").getString("code"))
            assertEquals("explode", envelope.getString("command"))
        }
    }

    @Test
    fun malformedJsonBodyReturns409() {
        server().use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status", body = "{not json")
            assertEquals(409, resp!!.status)
            assertEquals("BAD_REQUEST", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun oversizedContentLengthReturns409() {
        server(maxBodyBytes = 8).use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status", body = "{}")
            assertEquals(200, resp!!.status)
            val tooBig = exchange(
                srv.localPort,
                "POST /status HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nContent-Length: 64\r\n\r\n",
            )
            assertEquals(409, tooBig!!.status)
            assertEquals("BAD_REQUEST", JSONObject(tooBig.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun oversizedHeaderBlockClosesWithoutResponse() {
        server(maxHeaderBytes = 256).use { srv ->
            srv.start()
            val pad = "a".repeat(512)
            val resp = exchange(
                srv.localPort,
                "POST /status HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nX-Pad: $pad\r\nContent-Length: 0\r\n\r\n",
            )
            assertNull(resp)
            val after = post(srv.localPort, "/status")
            assertEquals(200, after!!.status)
        }
    }

    @Test
    fun headerTerminatorExactlyAtCapIsAccepted() {
        val fixed = "POST /status HTTP/1.1\r\n" +
            "Authorization: Bearer tok\r\n" +
            "X-Agent-Mobile-Version: 1\r\n" +
            "X-Pad: \r\n" +
            "Content-Length: 0\r\n" +
            "\r\n"
        val cap = 512
        val padLen = cap - fixed.length
        server(maxHeaderBytes = cap).use { srv ->
            srv.start()
            val atCap = exchange(
                srv.localPort,
                "POST /status HTTP/1.1\r\n" +
                    "Authorization: Bearer tok\r\n" +
                    "X-Agent-Mobile-Version: 1\r\n" +
                    "X-Pad: ${"a".repeat(padLen)}\r\n" +
                    "Content-Length: 0\r\n" +
                    "\r\n",
            )
            assertEquals(200, atCap!!.status)
            val overCap = exchange(
                srv.localPort,
                "POST /status HTTP/1.1\r\n" +
                    "Authorization: Bearer tok\r\n" +
                    "X-Agent-Mobile-Version: 1\r\n" +
                    "X-Pad: ${"a".repeat(padLen + 1)}\r\n" +
                    "Content-Length: 0\r\n" +
                    "\r\n",
            )
            assertNull(overCap)
        }
    }

    @Test
    fun closeDoesNotBlockOnStalledClient() {
        val srv = server(socketTimeoutMs = 10_000)
        srv.start()
        try {
            Socket("127.0.0.1", srv.localPort).use { stalled ->
                stalled.soTimeout = 3_000
                stalled.getOutputStream().apply {
                    write("POST /st".toByteArray(Charsets.UTF_8))
                    flush()
                }
                val deadline = System.currentTimeMillis() + 5_000
                while (!srv.hasActiveClient() && System.currentTimeMillis() < deadline) {
                    Thread.sleep(20)
                }
                assertTrue("server never registered the stalled client", srv.hasActiveClient())
                val startMs = System.currentTimeMillis()
                srv.close()
                val elapsedMs = System.currentTimeMillis() - startMs
                assertTrue("close blocked ${elapsedMs}ms", elapsedMs < 5_000)
                assertEquals(-1, stalled.getInputStream().read())
            }
        } finally {
            srv.close()
        }
    }

    @Test
    fun stalledClientTimesOutAndNextRequestSucceeds() {
        server(socketTimeoutMs = 300).use { srv ->
            srv.start()
            Socket("127.0.0.1", srv.localPort).use { stalled ->
                stalled.soTimeout = 2_000
                Thread.sleep(900)
            }
            val resp = post(srv.localPort, "/status")
            assertEquals(200, resp!!.status)
        }
    }

    @Test
    fun driverExceptionMapsTo409WithCode() {
        server(handler = { _, _ -> throw DriverException("STALE_REF", "gone") }).use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/tap")
            assertEquals(409, resp!!.status)
            assertEquals("STALE_REF", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun unexpectedHandlerExceptionMapsTo500DriverError() {
        server(handler = { _, _ -> throw IllegalStateException("boom") }).use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status")
            assertEquals(500, resp!!.status)
            assertEquals("DRIVER_ERROR", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun oldTokenRejectedAfterRestartWithNewToken() {
        val srv = server(token = "first-token")
        srv.start()
        assertEquals(200, post(srv.localPort, "/status", token = "first-token")!!.status)
        srv.close()

        server(token = "second-token").use { restarted ->
            restarted.start()
            assertEquals(401, post(restarted.localPort, "/status", token = "first-token")!!.status)
            assertEquals(200, post(restarted.localPort, "/status", token = "second-token")!!.status)
        }
    }

    @Test
    fun textPlainAcceptRendersSnapshotHeaderAndText() {
        val handler: (String, JSONObject) -> JSONObject = { _, _ ->
            JSONObject()
                .put("app", "com.x")
                .put("snapshot_id", "snap-9")
                .put("text", "line one\nline two")
        }
        server(handler = handler).use { srv ->
            srv.start()
            val resp = exchange(
                srv.localPort,
                "POST /snapshot HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nAccept: text/plain\r\nContent-Length: 0\r\n\r\n",
            )
            assertEquals(200, resp!!.status)
            assertEquals("text/plain", resp.headers["content-type"])
            assertTrue(resp.body.startsWith("app=com.x snapshot=@snap-9 refs=0 settled= reads= elapsed_ms="))
            assertTrue(resp.body.contains("line one\nline two\n"))
        }
    }

    @Test
    fun closeIsBounded() {
        val srv = server(socketTimeoutMs = 300)
        srv.start()
        val started = System.nanoTime()
        srv.close()
        val elapsedMs = (System.nanoTime() - started) / 1_000_000
        assertTrue("close took ${elapsedMs}ms", elapsedMs < 5_000)
    }
}
