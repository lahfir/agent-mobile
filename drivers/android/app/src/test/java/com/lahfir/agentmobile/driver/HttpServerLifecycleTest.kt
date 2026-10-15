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

class HttpServerLifecycleTest {

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

}
