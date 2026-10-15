package com.lahfir.agentmobile.driver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class SettlerTest {

    private val gapNanos = 150_000_000L
    private val timeoutNanos = 3_000_000_000L

    private fun node() = NodeModel(
        role = "group", name = "", value = "", states = emptyList(),
        availableActions = emptyList(), bounds = LogicalBounds(0.0, 0.0, 0.0, 0.0),
        nativeId = null, children = emptyList(),
        identity = NodeIdentity("", "", "", "", "", RawBounds(0, 0, 0, 0)),
    )

    private fun read(signature: String, atNanos: Long, complete: Boolean = true) =
        TreeRead("com.fake", node(), complete, signature, RawBounds(0, 0, 0, 0), 1.0, atNanos)

    private class Clock {
        var now: Long = 0
        fun time(): Long = now
        fun advance(nanos: Long) {
            now += nanos
        }
    }

    private fun settler(clock: Clock, sleeper: (Long) -> Unit = { clock.advance(it) }) =
        Settler(
            gapNanos = gapNanos,
            timeoutNanos = timeoutNanos,
            clock = clock::time,
            sleeper = sleeper,
        )

    @Test
    fun firstReadEqualToEarlierTreeDoesNotSettleAlone() {
        val clock = Clock()
        clock.now = 200_000_000
        var reads = 0
        val result = settler(clock).settle {
            reads += 1
            read("S", clock.time())
        }
        assertTrue(result.settled)
        assertEquals(2, reads)
        assertEquals(2, result.reads)
    }

    @Test
    fun missingWindowIsWaitedOutWhenTolerated() {
        val clock = Clock()
        var calls = 0
        val result = settler(clock).settle({ true }) {
            calls += 1
            if (calls == 1) throw DriverException("DRIVER_ERROR", "no window")
            read("S", clock.time())
        }
        assertTrue(result.settled)
        assertEquals(3, calls)
        assertEquals(2, result.reads)
    }

    @Test
    fun missingWindowFailsAtDeadline() {
        val clock = Clock()
        val e = runCatching {
            settler(clock).settle({ true }) { throw DriverException("DRIVER_ERROR", "no window") }
        }.exceptionOrNull()
        assertTrue(e is DriverException)
        assertTrue(clock.time() >= timeoutNanos)
    }

    @Test
    fun missingWindowFailsFastWhenNotTolerated() {
        val clock = Clock()
        var calls = 0
        val e = runCatching {
            settler(clock).settle { calls += 1; throw DriverException("DRIVER_ERROR", "no window") }
        }.exceptionOrNull()
        assertTrue(e is DriverException)
        assertEquals(1, calls)
    }

    @Test
    fun twoEqualReadsAtLeastGapApartSettle() {
        val clock = Clock()
        val result = settler(clock).settle { read("S", clock.time()) }
        assertTrue(result.settled)
        assertEquals(2, result.reads)
        assertTrue(result.settleMs >= 150)
    }

    @Test
    fun earlySleeperIsRetriedUntilFullGapElapses() {
        val clock = Clock()
        var sleepCalls = 0
        val readTimes = mutableListOf<Long>()
        val result = settler(clock) { clock.advance(50_000_000); sleepCalls += 1 }.settle {
            readTimes += clock.time()
            read("S", clock.time())
        }
        assertTrue(result.settled)
        assertTrue("early-wake sleeper must be retried", sleepCalls >= 3)
        assertEquals(2, result.reads)
        assertTrue(
            "second read must not start before the gap: ${readTimes[1] - readTimes[0]}",
            readTimes[1] - readTimes[0] >= gapNanos,
        )
    }

    @Test
    fun equalSignaturesWithTimestampsUnderGapDoNotSettle() {
        val wall = Clock()
        var platformTime = 0L
        val readTimes = mutableListOf<Long>()
        val sleeper = { nanos: Long -> wall.advance(nanos) }
        val result = settler(wall, sleeper).settle {
            readTimes += wall.time()
            read("S", platformTime)
        }
        assertFalse("equal sigs with close timestamps must not settle", result.settled)
        assertTrue(result.settleMs >= 3_000)
    }

    @Test
    fun nonAdvancingSleeperFailsBoundedly() {
        val clock = Clock()
        val e = runCatching {
            settler(clock) { }.settle { read("S", clock.time()) }
        }.exceptionOrNull()
        assertTrue(e is DriverException)
        assertEquals("DRIVER_ERROR", (e as DriverException).code)
    }

    @Test
    fun changingSignaturesStopAtDeadlineWithoutReadingAfter() {
        val clock = Clock()
        var i = 0
        val readTimes = mutableListOf<Long>()
        val result = settler(clock).settle {
            i += 1
            readTimes += clock.time()
            read("sig$i", clock.time())
        }
        assertFalse(result.settled)
        assertTrue(result.settleMs >= 3_000)
        assertTrue("reads must stop at the deadline, got ${result.reads}", result.reads <= 21)
        assertTrue(
            "no read may start after the deadline",
            readTimes.all { it <= clock.time() && it <= 3_000_000_000L },
        )
    }

    @Test
    fun interruptedSleepMapsDriverErrorAndPreservesInterrupt() {
        val clock = Clock()
        val result = runCatching {
            settler(clock) { throw InterruptedException("stop") }.settle { read("S", clock.time()) }
        }
        val e = result.exceptionOrNull()
        assertTrue(e is DriverException)
        assertEquals("DRIVER_ERROR", (e as DriverException).code)
        assertTrue(Thread.interrupted())
    }
}
