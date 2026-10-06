package com.lahfir.agentmobile.driver

import java.net.Socket
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class HttpServerTest {

    private fun exchange(port: Int, raw: String): HttpTestResponse? {
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

    private fun post(port: Int, path: String, token: String? = "tok", version: String? = "1", body: String = ""): HttpTestResponse? {
        val sb = StringBuilder("POST $path HTTP/1.1\r\n")
        sb.append("Host: 127.0.0.1\r\n")
        if (token != null) sb.append("Authorization: Bearer $token\r\n")
        if (version != null) sb.append("X-Agent-Mobile-Version: $version\r\n")
        sb.append("Content-Length: ${body.toByteArray(Charsets.UTF_8).size}\r\n")
        sb.append("Connection: close\r\n\r\n")
        sb.append(body)
        return exchange(port, sb.toString())
    }

    private fun statusHandler(app: String = "com.launcher"): (String, JSONObject, RequestCancellation) -> JSONObject =
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

    private fun server(
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
                val eof = try {
                    stalled.getInputStream().read()
                } catch (_: java.net.SocketException) {
                    -1
                }
                assertEquals(-1, eof)
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
    fun blockedResponseWriteClearsWithinDeadline() {
        val big = "x".repeat(9 * 1024 * 1024)
        val entered = java.util.concurrent.CountDownLatch(1)
        val handler: (String, JSONObject, RequestCancellation) -> JSONObject = { command, _, _ ->
            if (command == "snapshot") {
                entered.countDown()
                JSONObject().put("app", "com.x").put("snapshot_id", "s").put("text", big)
            } else {
                statusHandler()(command, JSONObject(), RequestCancellation())
            }
        }
        val srv = server(requestDeadlineMs = 300, handler = handler)
        srv.start()
        try {
            val socket = Socket("127.0.0.1", srv.localPort)
            socket.receiveBufferSize = 1024
            socket.sendBufferSize = 1024
            val request = "POST /snapshot HTTP/1.1\r\n" +
                "Authorization: Bearer tok\r\n" +
                "X-Agent-Mobile-Version: 1\r\n" +
                "Accept: text/plain\r\n" +
                "Content-Length: 0\r\n\r\n"
            socket.getOutputStream().write(request.toByteArray(Charsets.ISO_8859_1))
            socket.getOutputStream().flush()
            assertTrue("handler never ran", entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            assertTrue("first request must be active", srv.hasActiveClient())
            val limit = System.nanoTime() + 5_000_000_000L
            while (srv.hasActiveClient() && System.nanoTime() < limit) {
                Thread.sleep(50)
            }
            assertFalse("active request must clear once the deadline fires", srv.hasActiveClient())
            socket.close()
            val follow = post(srv.localPort, "/status")
            assertNotNull("a later request still succeeds", follow)
            assertEquals(200, follow!!.status)
        } finally {
            srv.close()
        }
    }

    @Test
    fun cancelledHandlerWritesNoResponseAfterDeadline() {
        val handler: (String, JSONObject, RequestCancellation) -> JSONObject = { _, _, _ ->
            Thread.sleep(2_000)
            JSONObject()
        }
        val srv = server(requestDeadlineMs = 300, handler = handler)
        srv.start()
        try {
            val socket = Socket("127.0.0.1", srv.localPort)
            socket.soTimeout = 3_000
            val request = "POST /status HTTP/1.1\r\n" +
                "Authorization: Bearer tok\r\n" +
                "X-Agent-Mobile-Version: 1\r\n" +
                "Content-Length: 0\r\n\r\n"
            socket.getOutputStream().write(request.toByteArray(Charsets.ISO_8859_1))
            socket.getOutputStream().flush()
            val first = try {
                socket.getInputStream().read()
            } catch (_: Exception) {
                -1
            }
            assertEquals(-1, first)
            socket.close()
            val limit = System.nanoTime() + 5_000_000_000L
            while (srv.hasActiveClient() && System.nanoTime() < limit) {
                Thread.sleep(50)
            }
            assertFalse(srv.hasActiveClient())
        } finally {
            srv.close()
        }
    }

    @Test
    fun elapsedIncludesHandlerWork() {
        var now = 1_000L
        val srv = server(
            clockMs = { now },
            handler = { _, _, _ ->
                now += 42
                JSONObject()
            },
        )
        srv.start()
        try {
            val response = post(srv.localPort, "/status")
            assertNotNull(response)
            val elapsed = JSONObject(response!!.body).getLong("elapsed_ms")
            assertTrue("elapsed $elapsed must include handler work", elapsed >= 42)
        } finally {
            srv.close()
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
    fun trickleBytesLoseToAbsoluteDeadline() {
        val srv = server(socketTimeoutMs = 30_000, requestDeadlineMs = 300)
        srv.start()
        try {
            Socket("127.0.0.1", srv.localPort).use { socket ->
                socket.soTimeout = 5_000
                val out = socket.getOutputStream()
                val start = System.nanoTime()
                var wrote = 0
                var killed = false
                while ((System.nanoTime() - start) / 1_000_000 < 3_000) {
                    try {
                        out.write("X".toByteArray())
                        out.flush()
                        wrote += 1
                    } catch (_: Exception) {
                        killed = true
                        break
                    }
                    Thread.sleep(10)
                }
                val dropped = try {
                    socket.getInputStream().read() < 0 || killed
                } catch (_: Exception) {
                    true
                }
                assertTrue("trickle must die by absolute deadline (wrote $wrote)", dropped)
            }
            val response = post(srv.localPort, "/status")
            assertNotNull("next request must still be served", response)
            assertEquals(200, response!!.status)
        } finally {
            srv.close()
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

    @Test
    fun closeCancelsActiveHandlerAndWaitsForExit() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val exited = java.util.concurrent.CountDownLatch(1)
        val clientResponded = java.util.concurrent.CountDownLatch(1)
        val srv = server(handler = { _, _, cancellation ->
            entered.countDown()
            try {
                while (!cancellation.isCancelled) {
                    Thread.sleep(5)
                }
                JSONObject()
            } finally {
                exited.countDown()
            }
        })
        srv.start()
        val requester = Thread {
            try {
                post(srv.localPort, "/status")?.let { clientResponded.countDown() }
            } catch (_: Exception) {
            }
        }
        requester.isDaemon = true
        requester.start()
        assertTrue("handler must start", entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
        val started = System.nanoTime()
        srv.close()
        val elapsedMs = (System.nanoTime() - started) / 1_000_000
        assertTrue("handler must observe cancellation and exit", exited.await(1, java.util.concurrent.TimeUnit.SECONDS))
        assertTrue("close must stay bounded", elapsedMs < 10_000)
        assertFalse("cancelled request must get no response", clientResponded.await(200, java.util.concurrent.TimeUnit.MILLISECONDS))
        assertFalse("listener worker must not survive close", srv.workerAlive())
    }

    @Test
    fun busyRequestsGet503WhileHandlerInFlight() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val calls = java.util.concurrent.atomic.AtomicInteger(0)
        val srv = server(handler = { _, _, _ ->
            calls.incrementAndGet()
            entered.countDown()
            release.await()
            JSONObject().put("app", "x")
        })
        srv.start()
        try {
            val firstDone = java.util.concurrent.CountDownLatch(1)
            Thread {
                post(srv.localPort, "/status")
                firstDone.countDown()
            }.apply { isDaemon = true }.start()
            assertTrue("first request must reach the handler", entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            repeat(2) { attempt ->
                val resp = post(srv.localPort, "/status")
                assertNotNull("attempt $attempt", resp)
                assertEquals(503, resp!!.status)
                val envelope = JSONObject(resp.body)
                assertFalse(envelope.getBoolean("ok"))
                assertEquals("DRIVER_ERROR", envelope.getJSONObject("error").getString("code"))
                assertFalse(envelope.has("command"))
                assertFalse(envelope.has("elapsed_ms"))
            }
            assertEquals("rejected requests must not invoke the handler", 1, calls.get())
            release.countDown()
            assertTrue("first request must finish", firstDone.await(5, java.util.concurrent.TimeUnit.SECONDS))
            var drained = false
            val deadline = System.nanoTime() + 5_000_000_000L
            while (System.nanoTime() < deadline) {
                if (!srv.hasActiveClient()) {
                    drained = true
                    break
                }
                Thread.sleep(10)
            }
            assertTrue("worker must fully exit before the next request", drained)
            val resp = post(srv.localPort, "/status")
            assertEquals(200, resp!!.status)
            assertEquals(2, calls.get())
        } finally {
            release.countDown()
            srv.close()
        }
    }

    @Test
    fun closeTerminatesStalledHeaderAndActiveHandler() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val srv = server(socketTimeoutMs = 300, handler = { _, _, cancellation ->
            entered.countDown()
            try {
                while (!cancellation.isCancelled) {
                    Thread.sleep(5)
                }
                JSONObject()
            } catch (_: InterruptedException) {
                JSONObject()
            }
        })
        srv.start()
        try {
            Thread {
                post(srv.localPort, "/status")
            }.apply { isDaemon = true }.start()
            assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            val stalled = java.net.Socket("127.0.0.1", srv.localPort)
            try {
                stalled.soTimeout = 3_000
                stalled.getOutputStream().write("POST /status HTTP/1.1\r\nHost: ".toByteArray(Charsets.UTF_8))
                stalled.getOutputStream().flush()
                srv.close()
                assertFalse("listener must be down", srv.workerAlive())
                val saw = try {
                    stalled.getInputStream().read()
                } catch (_: java.net.SocketException) {
                    -1
                }
                assertTrue("stalled client must see close", saw < 0)
            } finally {
                try {
                    stalled.close()
                } catch (_: java.io.IOException) {
                }
            }
        } finally {
            srv.close()
        }
    }

    @Test
    fun stubbornWorkerKeepsRefsAndBlocksRestartUntilReleased() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val srv = server(
            socketTimeoutMs = 50,
            clockMs = { 0L },
            handler = { _, _, _ ->
                entered.countDown()
                // Ignore every interrupt until the test releases the latch —
                // proves the close bound does not hang on a dead clock.
                while (true) {
                    try {
                        if (release.await(1, java.util.concurrent.TimeUnit.MILLISECONDS)) {
                            break
                        }
                    } catch (_: InterruptedException) {
                    }
                }
                JSONObject().put("app", "x")
            },
        )
        srv.start()
        try {
            Thread {
                post(srv.localPort, "/status")
            }.apply { isDaemon = true }.start()
            assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            val started = System.nanoTime()
            srv.close()
            val closeMs = (System.nanoTime() - started) / 1_000_000
            assertTrue("close must stay bounded by real time (took ${closeMs}ms)", closeMs < 3_000)
            assertTrue("stubborn worker must keep refs alive", srv.workerAlive())
            try {
                srv.start()
                fail("closed server must not restart")
            } catch (_: IllegalStateException) {
            }
            release.countDown()
            var drained = false
            val deadline = System.nanoTime() + 5_000_000_000L
            while (System.nanoTime() < deadline) {
                if (!srv.workerAlive()) {
                    drained = true
                    break
                }
                Thread.sleep(10)
            }
            assertTrue("worker must die after release", drained)
            srv.close()
            assertFalse("second close must clear retained refs", srv.workerAlive())
        } finally {
            release.countDown()
            srv.close()
        }
    }

    @Test
    fun secondListenerGets503WhileSharedDriverIsBlocked() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val gateOpen = java.util.concurrent.atomic.AtomicBoolean(false)
        val node = NodeModel(
            role = "group", name = "", value = "", states = emptyList(),
            availableActions = emptyList(),
            bounds = LogicalBounds(0.0, 0.0, 100.0, 100.0),
            nativeId = null, children = emptyList(),
            identity = NodeIdentity(
                "android.widget.FrameLayout", "group", "", "", "",
                RawBounds(0, 0, 100, 100), packageName = "com.fake",
            ),
        )
        val driver = Driver(
            object : DriverPlatform {
                override fun status(): PlatformStatus = PlatformStatus("com.fake", "emu64a", "17")
                override fun readTree(): TreeRead {
                    if (!gateOpen.get()) {
                        return TreeRead(
                            "com.fake", node, true, "sig",
                            node.identity.rawBounds, 1.0, 0L,
                        )
                    }
                    entered.countDown()
                    try {
                        release.await()
                    } catch (_: InterruptedException) {
                        throw java.util.concurrent.CancellationException("read interrupted")
                    }
                    throw DriverException("DRIVER_ERROR", "unreachable")
                }
                override fun performNodeAction(
                    target: RefTarget,
                    action: NodeAction,
                    cancellation: RequestCancellation,
                ): NodeActionResult = throw DriverException("DRIVER_ERROR", "unused")
                override fun appendToFocused(
                    text: String,
                    cancellation: RequestCancellation,
                ): NodeActionResult = throw DriverException("DRIVER_ERROR", "unused")
                override fun dispatchGesture(spec: GestureSpec) {}
                override fun performGlobal(action: GlobalAction): Boolean = true
                override fun screenshot(): ByteArray = byteArrayOf()
            },
        )
        val primedSnapshotId = driver.handle("snapshot", JSONObject()).getString("snapshot_id")
        assertTrue("primed snapshot must have a real id", primedSnapshotId.isNotEmpty())
        gateOpen.set(true)
        val oldSrv = server(handler = { command, params, cancellation ->
            driver.handle(command, params, cancellation)
        })
        val newSrv = server(handler = { command, params, cancellation ->
            driver.handle(command, params, cancellation)
        })
        oldSrv.start()
        newSrv.start()
        try {
            val oldReplied = java.util.concurrent.CountDownLatch(1)
            val oldWorker = Thread {
                try {
                    if (post(oldSrv.localPort, "/snapshot", body = "{}") != null) {
                        oldReplied.countDown()
                    }
                } catch (_: Exception) {
                }
            }
            oldWorker.isDaemon = true
            oldWorker.start()
            assertTrue("first listener request must enter driver", entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            val resp = post(newSrv.localPort, "/status")
            assertNotNull(resp)
            assertEquals(503, resp!!.status)
            val envelope = JSONObject(resp.body)
            assertEquals("DRIVER_ERROR", envelope.getJSONObject("error").getString("code"))
            oldSrv.close()
            assertFalse("closed listener must not answer the held request", oldReplied.await(200, java.util.concurrent.TimeUnit.MILLISECONDS))
            oldWorker.join(5_000)
            val next = post(newSrv.localPort, "/status")
            assertNotNull(next)
            assertEquals(200, next!!.status)
            val nextBody = JSONObject(next.body).getJSONObject("data")
            assertEquals(primedSnapshotId, nextBody.getString("snapshot_id"))
        } finally {
            release.countDown()
            oldSrv.close()
            newSrv.close()
        }
    }

    @Test
    fun peerResetAfterHandlerEntryStaysContainedInWorker() {
        val escaped = java.util.concurrent.ConcurrentLinkedQueue<Throwable>()
        val original = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { t, e ->
            if (t.name.startsWith("agent-mobile-http")) escaped.add(e)
            else original?.uncaughtException(t, e)
        }
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val srv = server(handler = { _, _, _ ->
            entered.countDown()
            release.await()
            JSONObject().put("app", "x")
        })
        srv.start()
        try {
            val client = java.net.Socket("127.0.0.1", srv.localPort)
            client.soTimeout = 3_000
            try {
                client.getOutputStream().apply {
                    write("POST /status HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nContent-Length: 0\r\n\r\n".toByteArray(Charsets.UTF_8))
                    flush()
                }
                assertTrue("handler must enter", entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                client.setSoLinger(true, 0)
                client.close()
                release.countDown()
                var seen = false
                val deadline = System.nanoTime() + 5_000_000_000L
                while (System.nanoTime() < deadline) {
                    if (!srv.hasActiveClient()) { seen = true; break }
                    Thread.sleep(10)
                }
                assertTrue("worker must exit after peer reset", seen)
                val good = post(srv.localPort, "/status")
                assertNotNull("next request must succeed", good)
                assertEquals(200, good!!.status)
            } finally {
                try { client.close() } catch (_: Exception) {}
                release.countDown()
            }
        } finally {
            srv.close()
            Thread.setDefaultUncaughtExceptionHandler(original)
        }
        assertTrue("nothing may escape the worker: $escaped", escaped.isEmpty())
    }

    @Test
    fun startVsCloseRaceLeavesNoWorkerAndRefusesRestart() {
        val handlerCalls = java.util.concurrent.atomic.AtomicInteger(0)
        val srv = server(handler = { _, _, _ ->
            handlerCalls.incrementAndGet()
            JSONObject().put("app", "x")
        })
        val barrier = java.util.concurrent.CountDownLatch(1)
        val starterFailure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val closerFailure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val starter = Thread {
            try {
                barrier.await()
                srv.start()
            } catch (e: IllegalStateException) {
                starterFailure.set(e)
            } catch (t: Throwable) {
                starterFailure.set(t)
            }
        }
        val closer = Thread {
            try {
                barrier.await()
                srv.close()
            } catch (t: Throwable) {
                closerFailure.set(t)
            }
        }
        starter.isDaemon = true
        closer.isDaemon = true
        starter.start()
        closer.start()
        barrier.countDown()
        starter.join(5_000)
        closer.join(5_000)
        try {
            assertFalse("starter must finish", starter.isAlive)
            assertFalse("closer must finish", closer.isAlive)
            starterFailure.get()?.let {
                assertTrue("starter may only fail with restart denial", it is IllegalStateException)
                assertEquals("closed server cannot be restarted", it.message)
            }
            assertNull("closer must not fail", closerFailure.get())
            assertFalse("no worker may survive", srv.workerAlive())
            assertEquals("listener must be released", -1, srv.localPort)
            try {
                srv.start()
                fail("post-close restart must be denied")
            } catch (e: IllegalStateException) {
                assertEquals("closed server cannot be restarted", e.message)
            }
            assertEquals("closed admission must never dispatch", 0, handlerCalls.get())
        } finally {
            srv.close()
        }
    }

    @Test
    fun busyReplyDoesNotMonopolizeAcceptorDuringFlood() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val calls = java.util.concurrent.atomic.AtomicInteger(0)
        val srv = server(socketTimeoutMs = 200, handler = { _, _, _ ->
            calls.incrementAndGet()
            entered.countDown()
            release.await()
            JSONObject().put("app", "x")
        })
        srv.start()
        try {
            val firstDone = java.util.concurrent.CountDownLatch(1)
            Thread {
                post(srv.localPort, "/status")
                firstDone.countDown()
            }.apply { isDaemon = true }.start()
            assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            val flood = java.net.Socket("127.0.0.1", srv.localPort)
            try {
                flood.soTimeout = 2_000
                flood.getOutputStream().apply {
                    write("POST /status HTTP/1.1\r\n".toByteArray(Charsets.UTF_8))
                    flush()
                }
                val trickler = Thread {
                    try {
                        repeat(5_000) {
                            flood.getOutputStream().write('a'.code)
                            flood.getOutputStream().flush()
                            Thread.sleep(1)
                        }
                    } catch (_: Exception) {
                    }
                }
                trickler.isDaemon = true
                trickler.start()
                val resp = StringBuilder()
                val buf = ByteArray(4096)
                try {
                    while (true) {
                        val n = flood.getInputStream().read(buf)
                        if (n < 0) break
                        resp.append(String(buf, 0, n, Charsets.ISO_8859_1))
                        if (resp.contains("\r\n\r\n")) break
                    }
                } catch (_: Exception) {
                }
                assertTrue("busy client must get 503: $resp", resp.toString().startsWith("HTTP/1.1 503"))
                val third = post(srv.localPort, "/status")
                assertNotNull("a further bounded request must also get a bounded 503", third)
                assertEquals(503, third!!.status)
                assertEquals("flooded rejections must never dispatch", 1, calls.get())
            } finally {
                try { flood.close() } catch (_: Exception) {}
            }
            release.countDown()
            assertTrue(firstDone.await(5, java.util.concurrent.TimeUnit.SECONDS))
        } finally {
            release.countDown()
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
