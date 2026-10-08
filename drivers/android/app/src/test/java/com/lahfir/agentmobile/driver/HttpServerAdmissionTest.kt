package com.lahfir.agentmobile.driver

import java.net.Socket
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HttpServerAdmissionTest {

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
    fun completedResponseAdmitsNextRequestWhileWatchdogRetires() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val firstWatcher = java.util.concurrent.atomic.AtomicBoolean(false)
        val clock: () -> Long = {
            if (Thread.currentThread().name == "agent-mobile-http-deadline" &&
                firstWatcher.compareAndSet(false, true)
            ) {
                entered.countDown()
                awaitUninterruptibly(release)
            }
            System.nanoTime() / 1_000_000
        }
        val srv = server(
            clockMs = clock,
            handler = { _, _, _ ->
                assertTrue("first watcher must be blocked before handler returns",
                    entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                JSONObject().put("app", "x")
            },
        )
        srv.start()
        try {
            val first = post(srv.localPort, "/status")
            assertNotNull(first)
            assertEquals(200, first!!.status)
            val second = post(srv.localPort, "/status")
            assertNotNull(second)
            assertEquals("response complete but retiring watchdog must not block next request",
                200, second!!.status)
        } finally {
            release.countDown()
            srv.close()
        }
    }

    @Test
    fun queuedRequestExpiresBeforeDispatch() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val firstWatcher = java.util.concurrent.atomic.AtomicBoolean(false)
        val calls = java.util.concurrent.atomic.AtomicInteger(0)
        val clock: () -> Long = {
            if (Thread.currentThread().name == "agent-mobile-http-deadline" &&
                firstWatcher.compareAndSet(false, true)
            ) {
                entered.countDown()
                awaitUninterruptibly(release)
            }
            System.nanoTime() / 1_000_000
        }
        val srv = server(
            requestDeadlineMs = 300,
            clockMs = clock,
            handler = { _, _, _ ->
                calls.incrementAndGet()
                assertTrue("first watcher must be blocked before handler returns",
                    entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                JSONObject().put("app", "x")
            },
        )
        srv.start()
        try {
            val first = post(srv.localPort, "/status")
            assertNotNull(first)
            assertEquals(200, first!!.status)
            val queued = java.net.Socket("127.0.0.1", srv.localPort)
            try {
                queued.soTimeout = 3_000
                queued.getOutputStream().apply {
                    write("POST /status HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Agent-Mobile-Version: 1\r\nContent-Length: 0\r\n\r\n".toByteArray(Charsets.ISO_8859_1))
                    flush()
                }
                val saw = try {
                    queued.getInputStream().read()
                } catch (e: java.net.SocketException) {
                    -1
                }
                assertEquals("queued request must expire to EOF/reset, not be served", -1, saw)
            } finally {
                try {
                    queued.close()
                } catch (_: java.io.IOException) {
                }
            }
            assertEquals("expired queued work must never reach the handler", 1, calls.get())
        } finally {
            release.countDown()
            srv.close()
            assertEquals(1, calls.get())
        }
    }
}
