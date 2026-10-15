package com.lahfir.agentmobile.driver

import java.net.Socket
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HttpServerFramingTest {

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
    fun oversizedHeaderBlockReturnsBadRequest() {
        server(maxHeaderBytes = 256).use { srv ->
            srv.start()
            val pad = "a".repeat(512)
            val resp = exchange(
                srv.localPort,
                "POST /status HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nX-Pad: $pad\r\nContent-Length: 0\r\n\r\n",
            )
            assertNotNull(resp)
            assertEquals(409, resp!!.status)
            val env = JSONObject(resp.body)
            assertFalse(env.getBoolean("ok"))
            assertEquals("BAD_REQUEST", env.getJSONObject("error").getString("code"))
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
            assertNotNull(overCap)
            assertEquals(409, overCap!!.status)
            val env = JSONObject(overCap.body)
            assertEquals("BAD_REQUEST", env.getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun driverExceptionMapsTo409WithCode() {
        server(handler = { _, _, _ -> throw DriverException("STALE_REF", "gone") }).use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/tap")
            assertEquals(409, resp!!.status)
            assertEquals("STALE_REF", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun unexpectedHandlerExceptionMapsTo500DriverError() {
        server(handler = { _, _, _ -> throw IllegalStateException("boom") }).use { srv ->
            srv.start()
            val resp = post(srv.localPort, "/status")
            assertEquals(500, resp!!.status)
            assertEquals("DRIVER_ERROR", JSONObject(resp.body).getJSONObject("error").getString("code"))
        }
    }

    @Test
    fun textPlainAcceptRendersSnapshotHeaderAndText() {
        val handler: (String, JSONObject, RequestCancellation) -> JSONObject = { _, _, _ ->
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
            assertFalse("no marker for a missing/complete snapshot", resp.body.contains("complete"))
        }
    }

    @Test
    fun textPlainAcceptMarksIncompleteSnapshot() {
        val handler: (String, JSONObject, RequestCancellation) -> JSONObject = { _, _, _ ->
            JSONObject()
                .put("app", "com.x")
                .put("snapshot_id", "snap-9")
                .put("complete", false)
                .put("text", "partial")
        }
        server(handler = handler).use { srv ->
            srv.start()
            val resp = exchange(
                srv.localPort,
                "POST /snapshot HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nAccept: text/plain\r\nContent-Length: 0\r\n\r\n",
            )
            assertEquals(200, resp!!.status)
            assertTrue(resp.body, resp.body.contains(" complete=false\n"))
            assertTrue(resp.body.endsWith("partial\n"))
        }
    }

    @Test
    fun driverErrorMapsTo500() {
        val srv = server(
            handler = { _, _, _ -> throw DriverException("DRIVER_ERROR", "boom") },
        )
        srv.start()
        try {
            val response = post(srv.localPort, "/status")
            assertNotNull(response)
            assertEquals(500, response!!.status)
            val error = JSONObject(response.body).getJSONObject("error")
            assertEquals("DRIVER_ERROR", error.getString("code"))
        } finally {
            srv.close()
        }
    }

    @Test
    fun staleRefStays409() {
        val srv = server(
            handler = { _, _, _ -> throw DriverException("STALE_REF", "gone") },
        )
        srv.start()
        try {
            val response = post(srv.localPort, "/tap", body = "{}")
            assertNotNull(response)
            assertEquals(409, response!!.status)
        } finally {
            srv.close()
        }
    }

    @Test
    fun incompleteHeadAtCapFailsFastWith409() {
        server(maxHeaderBytes = 256).use { srv ->
            srv.start()
            val prefix = "POST /status HTTP/1.1\r\n" +
                "Authorization: Bearer tok\r\n" +
                "X-Agent-Mobile-Version: 1\r\n" +
                "X-Pad: "
            val raw = prefix + "a".repeat(256 - prefix.length)
            assertEquals(256, raw.toByteArray(Charsets.UTF_8).size)
            val client = java.net.Socket("127.0.0.1", srv.localPort)
            try {
                client.soTimeout = 3_000
                val started = System.nanoTime()
                client.getOutputStream().apply {
                    write(raw.toByteArray(Charsets.UTF_8))
                    flush()
                }
                val buf = ByteArray(512)
                val n = client.getInputStream().read(buf)
                val elapsedMs = (System.nanoTime() - started) / 1_000_000
                assertTrue("must answer without waiting for more bytes", n > 0)
                val reply = String(buf, 0, n, Charsets.ISO_8859_1)
                assertTrue("must be 409: $reply", reply.startsWith("HTTP/1.1 409"))
                assertTrue("fail-fast must be bounded", elapsedMs < 3_000)
            } finally {
                try { client.close() } catch (_: Exception) {}
            }
            val after = post(srv.localPort, "/status")
            assertEquals(200, after!!.status)
        }
    }

}
