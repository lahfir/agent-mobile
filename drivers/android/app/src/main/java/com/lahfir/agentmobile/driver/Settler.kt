package com.lahfir.agentmobile.driver

import java.util.concurrent.TimeUnit

internal data class SettleResult(
    val read: TreeRead,
    val settled: Boolean,
    val reads: Int,
    val settleMs: Long,
)

internal class Settler(
    private val gapNanos: Long = TimeUnit.MILLISECONDS.toNanos(150),
    private val timeoutNanos: Long = TimeUnit.SECONDS.toNanos(3),
    private val clock: () -> Long = System::nanoTime,
    private val sleeper: (Long) -> Unit = { nanos ->
        Thread.sleep(nanos / 1_000_000, (nanos % 1_000_000).toInt())
    },
) {

    /** [tolerate] marks read failures worth waiting out until the deadline, e.g. a window still opening;
     *  a tolerated gap between two reads means they cannot prove stability. */
    fun settle(
        tolerate: (DriverException) -> Boolean = { false },
        read: () -> TreeRead,
    ): SettleResult {
        val start = clock()
        val deadline = start + timeoutNanos
        var interrupted = false
        fun attempt(): TreeRead {
            while (true) {
                try {
                    return read()
                } catch (e: DriverException) {
                    if (!tolerate(e) || clock() >= deadline) {
                        throw e
                    }
                    interrupted = true
                    waitUntil((clock() + gapNanos).coerceAtMost(deadline))
                }
            }
        }
        var previous = attempt()
        var reads = 1
        var lastReadWall = clock()
        while (true) {
            val wakeAt = (maxOf(previous.readAtNanos, lastReadWall) + gapNanos).coerceAtMost(deadline)
            val now = waitUntil(wakeAt)
            if (now >= deadline) {
                return SettleResult(previous, false, reads, elapsedMs(start))
            }
            interrupted = false
            val next = attempt()
            reads += 1
            lastReadWall = clock()
            if (!interrupted &&
                next.signature == previous.signature &&
                next.readAtNanos - previous.readAtNanos >= gapNanos
            ) {
                return SettleResult(next, true, reads, elapsedMs(start))
            }
            previous = next
        }
    }

    private fun waitUntil(wakeAt: Long): Long {
        var now = clock()
        while (now < wakeAt) {
            sleepGap(wakeAt - now)
            val progressed = clock()
            if (progressed <= now) {
                throw DriverException("DRIVER_ERROR", "settle sleeper returned with no clock progress")
            }
            now = progressed
        }
        return now
    }

    private fun sleepGap(nanos: Long) {
        try {
            sleeper(nanos)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            throw DriverException("DRIVER_ERROR", "settle interrupted")
        }
    }

    private fun elapsedMs(start: Long): Long =
        ((clock() - start) / 1_000_000).coerceAtLeast(0)
}
