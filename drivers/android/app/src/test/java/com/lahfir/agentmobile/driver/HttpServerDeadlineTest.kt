package com.lahfir.agentmobile.driver

import java.net.Socket
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HttpServerDeadlineTest {

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
    fun retiredDeadlineCannotInterruptNextRequest() {
        val firstWatcherEntered = java.util.concurrent.CountDownLatch(1)
        val releaseFirst = java.util.concurrent.CountDownLatch(1)
        val secondEntered = java.util.concurrent.CountDownLatch(1)
        val releaseSecond = java.util.concurrent.CountDownLatch(1)
        val firstWatcherCalled = java.util.concurrent.atomic.AtomicBoolean(false)
        val watcher1 = java.util.concurrent.atomic.AtomicReference<Thread?>()
        val clock: () -> Long = {
            if (Thread.currentThread().name == "agent-mobile-http-deadline" &&
                firstWatcherCalled.compareAndSet(false, true)
            ) {
                watcher1.set(Thread.currentThread())
                firstWatcherEntered.countDown()
                awaitUninterruptibly(releaseFirst)
                Long.MAX_VALUE
            } else {
                System.nanoTime() / 1_000_000
            }
        }
        val calls = java.util.concurrent.atomic.AtomicInteger(0)
        val secondInterrupted = java.util.concurrent.atomic.AtomicBoolean(false)
        val secondCancellation = java.util.concurrent.atomic.AtomicReference<RequestCancellation?>()
        val srv = server(
            clockMs = clock,
            handler = { _, _, cancellation ->
                val call = calls.incrementAndGet()
                if (call == 1) {
                    assertTrue("first watcher must be armed before handler returns",
                        firstWatcherEntered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                } else {
                    secondCancellation.set(cancellation)
                    secondEntered.countDown()
                    try {
                        if (!releaseSecond.await(5, java.util.concurrent.TimeUnit.SECONDS)) {
                            throw DriverException("DRIVER_ERROR", "test did not release")
                        }
                    } catch (e: InterruptedException) {
                        secondInterrupted.set(true)
                        throw DriverException("DRIVER_ERROR", "interrupted")
                    }
                    secondInterrupted.set(Thread.currentThread().isInterrupted)
                    cancellation.check()
                }
                JSONObject().put("app", "x")
            },
        )
        srv.start()
        val respRef = java.util.concurrent.atomic.AtomicReference<HttpTestResponse?>()
        val failureRef = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val requester = Thread {
            try {
                respRef.set(post(srv.localPort, "/status"))
            } catch (t: Throwable) {
                failureRef.set(t)
            }
        }
        try {
            val first = post(srv.localPort, "/status")
            assertNotNull(first)
            assertEquals(200, first!!.status)
            requester.isDaemon = true
            requester.start()
            assertTrue("second handler must enter", secondEntered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            releaseFirst.countDown()
            val w = watcher1.get()
            assertNotNull(w)
            w!!.join(5_000)
            assertFalse("first watcher must be fully retired", w.isAlive)
            assertFalse("retired watcher must not interrupt the live handler", secondInterrupted.get())
            releaseSecond.countDown()
            requester.join(5_000)
            assertFalse("second request must finish", requester.isAlive)
            assertNull("requester must not fail", failureRef.get())
            val second = respRef.get()
            assertNotNull(second)
            assertEquals(200, second!!.status)
            assertFalse("second handler must not observe interrupt", secondInterrupted.get())
            assertFalse("second cancellation must stay clean",
                secondCancellation.get()?.isCancelled ?: true)
        } finally {
            releaseFirst.countDown()
            releaseSecond.countDown()
            requester.join(5_000)
            srv.close()
        }
    }

    @Test
    fun closeRetainsBlockedDeadlineWatcher() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val watcher = java.util.concurrent.atomic.AtomicReference<Thread?>()
        val firstWatcher = java.util.concurrent.atomic.AtomicBoolean(false)
        val clock: () -> Long = {
            if (Thread.currentThread().name == "agent-mobile-http-deadline" &&
                firstWatcher.compareAndSet(false, true)
            ) {
                watcher.set(Thread.currentThread())
                entered.countDown()
                awaitUninterruptibly(release)
            }
            System.nanoTime() / 1_000_000
        }
        val srv = server(
            socketTimeoutMs = 50,
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
            assertEquals(200, second!!.status)
            val started = System.nanoTime()
            srv.close()
            val closeMs = (System.nanoTime() - started) / 1_000_000
            assertTrue("close must stay bounded (took ${closeMs}ms)", closeMs < 3_000)
            val w = watcher.get()
            assertNotNull(w)
            assertTrue("blocked watcher must stay tracked", w!!.isAlive)
            assertTrue("live watcher must count as owned thread", srv.workerAlive())
            release.countDown()
            w.join(5_000)
            assertFalse(w.isAlive)
            srv.close()
            assertFalse("all owned threads must be retired", srv.workerAlive())
        } finally {
            release.countDown()
            srv.close()
        }
    }

}
