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

    fun settle(baseline: TreeRead? = null, read: () -> TreeRead): SettleResult {
        val start = clock()
        val deadline = start + timeoutNanos
        var previous = read()
        var reads = 1
        if (baseline != null &&
            baseline.signature == previous.signature &&
            previous.readAtNanos - baseline.readAtNanos >= gapNanos
        ) {
            return SettleResult(previous, true, reads, elapsedMs(start))
        }
        var lastReadWall = clock()
        while (true) {
            val wakeAt = (maxOf(previous.readAtNanos, lastReadWall) + gapNanos).coerceAtMost(deadline)
            var now = clock()
            while (now < wakeAt) {
                sleepGap(wakeAt - now)
                val progressed = clock()
                if (progressed <= now) {
                    throw DriverException("DRIVER_ERROR", "settle sleeper returned with no clock progress")
                }
                now = progressed
            }
            if (now >= deadline) {
                return SettleResult(previous, false, reads, elapsedMs(start))
            }
            val next = read()
            reads += 1
            lastReadWall = clock()
            if (next.signature == previous.signature &&
                next.readAtNanos - previous.readAtNanos >= gapNanos
            ) {
                return SettleResult(next, true, reads, elapsedMs(start))
            }
            previous = next
        }
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
