package com.lahfir.agentmobile.driver

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.Handler
import android.os.Looper
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.abs

internal data class RawPoint(val x: Float, val y: Float)
internal data class StrokeSpec(val points: List<RawPoint>, val startMs: Long, val durationMs: Long)
internal data class GestureSpec(val strokes: List<StrokeSpec>)

internal object Gestures {

    fun toRaw(logical: RawPoint, read: TreeRead): RawPoint = RawPoint(
        read.origin.left + logical.x * read.density.toFloat(),
        read.origin.top + logical.y * read.density.toFloat(),
    )

    fun rawCenter(bounds: RawBounds): RawPoint {
        requireUsable(bounds)
        return RawPoint(bounds.left + bounds.width / 2f, bounds.top + bounds.height / 2f)
    }

    fun tap(center: RawPoint): GestureSpec =
        GestureSpec(listOf(StrokeSpec(listOf(center), startMs = 0, durationMs = 50)))

    fun doubleTap(center: RawPoint): GestureSpec = GestureSpec(
        listOf(
            StrokeSpec(listOf(center), startMs = 0, durationMs = 50),
            StrokeSpec(listOf(center), startMs = 150, durationMs = 50),
        ),
    )

    fun hold(center: RawPoint, durationMs: Long): GestureSpec =
        GestureSpec(listOf(StrokeSpec(listOf(center), startMs = 0, durationMs = durationMs)))

    fun swipe(bounds: RawBounds, direction: String, span: Double): GestureSpec {
        requireUsable(bounds)
        val center = rawCenter(bounds)
        val dx = when (direction) {
            "left" -> -bounds.width * span
            "right" -> bounds.width * span
            else -> 0.0
        }
        val dy = when (direction) {
            "up" -> -bounds.height * span
            "down" -> bounds.height * span
            else -> 0.0
        }
        val from = RawPoint((center.x - dx / 2).toFloat(), (center.y - dy / 2).toFloat())
        val to = RawPoint((center.x + dx / 2).toFloat(), (center.y + dy / 2).toFloat())
        return GestureSpec(listOf(StrokeSpec(interpolate(from, to, 6), startMs = 0, durationMs = 200)))
    }

    fun pinch(bounds: RawBounds, scale: Double, velocity: Double): GestureSpec {
        requireUsable(bounds)
        val horizontal = bounds.width >= bounds.height
        val half = (if (horizontal) bounds.width else bounds.height) / 2.0
        val maxOffset = half - 1.0
        if (maxOffset <= 1.0) {
            throw DriverException("DRIVER_ERROR", "element bounds too small for pinch")
        }
        val (fromOffset, toOffset) = if (scale > 1.0) {
            (maxOffset / scale).coerceAtLeast(1.0) to maxOffset
        } else {
            maxOffset to (maxOffset * scale).coerceAtLeast(1.0)
        }
        val center = rawCenter(bounds)
        val startMs = 0L
        val durationMs = if (velocity == 0.0) {
            300L
        } else {
            (400.0 / abs(velocity)).toLong().coerceIn(100, 1_000)
        }
        fun offsetPoint(offset: Double): RawPoint = if (horizontal) {
            RawPoint((center.x + offset).toFloat(), center.y)
        } else {
            RawPoint(center.x, (center.y + offset).toFloat())
        }
        return GestureSpec(
            listOf(
                StrokeSpec(interpolate(offsetPoint(-fromOffset), offsetPoint(-toOffset), 6), startMs, durationMs),
                StrokeSpec(interpolate(offsetPoint(fromOffset), offsetPoint(toOffset), 6), startMs, durationMs),
            ),
        )
    }

    fun twoFinger(bounds: RawBounds): GestureSpec {
        requireUsable(bounds)
        val horizontal = bounds.width >= bounds.height
        val center = rawCenter(bounds)
        val extent = (if (horizontal) bounds.width else bounds.height) / 4.0
        if (extent <= 1.0) {
            throw DriverException("DRIVER_ERROR", "element bounds too small for two-finger tap")
        }
        val first = if (horizontal) RawPoint(center.x - extent.toFloat(), center.y) else RawPoint(center.x, center.y - extent.toFloat())
        val second = if (horizontal) RawPoint(center.x + extent.toFloat(), center.y) else RawPoint(center.x, center.y + extent.toFloat())
        return GestureSpec(
            listOf(
                StrokeSpec(listOf(first), startMs = 0, durationMs = 100),
                StrokeSpec(listOf(second), startMs = 0, durationMs = 100),
            ),
        )
    }

    private fun requireUsable(bounds: RawBounds) {
        if (bounds.width <= 0 || bounds.height <= 0) {
            throw DriverException("DRIVER_ERROR", "element has no usable bounds")
        }
    }

    private fun interpolate(from: RawPoint, to: RawPoint, steps: Int): List<RawPoint> =
        (0 until steps).map { i ->
            val t = if (steps == 1) 0f else i.toFloat() / (steps - 1)
            RawPoint(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)
        }
}

internal fun interface GestureSubmission {
    fun submit(spec: GestureSpec, complete: (Boolean) -> Unit): Boolean
}

internal class GestureDispatcher(
    private val submission: GestureSubmission,
    private val timeoutMarginMs: Long = 5_000,
    private val maxStrokes: Int = 10,
    private val maxDurationMs: Long = 60_000,
) {

    fun dispatch(spec: GestureSpec) {
        validate(spec)
        val latch = CountDownLatch(1)
        val completed = AtomicBoolean()
        val accepted = try {
            submission.submit(spec) { ok ->
                completed.set(ok)
                latch.countDown()
            }
        } catch (e: Exception) {
            throw DriverException("DRIVER_ERROR", "gesture dispatch failed: ${e.javaClass.simpleName}")
        }
        if (!accepted) {
            throw DriverException("DRIVER_ERROR", "gesture dispatch refused")
        }
        val totalMs = spec.strokes.maxOf { it.startMs + it.durationMs }
        val arrived = try {
            latch.await(totalMs + timeoutMarginMs, TimeUnit.MILLISECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            throw DriverException("DRIVER_ERROR", "gesture wait interrupted")
        }
        if (!arrived) {
            throw DriverException("DRIVER_ERROR", "gesture callback timed out")
        }
        if (!completed.get()) {
            throw DriverException("DRIVER_ERROR", "gesture cancelled")
        }
    }

    private fun validate(spec: GestureSpec) {
        if (spec.strokes.isEmpty() || spec.strokes.size > maxStrokes) {
            throw DriverException("BAD_REQUEST", "invalid gesture spec")
        }
        spec.strokes.forEach { stroke ->
            if (stroke.points.isEmpty() || stroke.startMs < 0 || stroke.durationMs <= 0) {
                throw DriverException("BAD_REQUEST", "invalid gesture spec")
            }
            if (stroke.startMs + stroke.durationMs > maxDurationMs) {
                throw DriverException("BAD_REQUEST", "invalid gesture spec")
            }
            if (stroke.points.any { !it.x.isFinite() || !it.y.isFinite() }) {
                throw DriverException("BAD_REQUEST", "invalid gesture spec")
            }
        }
    }
}

internal class AndroidGestureSubmission(
    private val service: AgentMobileAccessibilityService,
) : GestureSubmission {

    override fun submit(spec: GestureSpec, complete: (Boolean) -> Unit): Boolean {
        val builder = GestureDescription.Builder()
        spec.strokes.forEach { stroke ->
            val path = Path()
            stroke.points.forEachIndexed { index, point ->
                if (index == 0) path.moveTo(point.x, point.y) else path.lineTo(point.x, point.y)
            }
            builder.addStroke(GestureDescription.StrokeDescription(path, stroke.startMs, stroke.durationMs))
        }
        return service.onMain {
            service.dispatchGesture(
                builder.build(),
                object : AccessibilityService.GestureResultCallback() {
                    override fun onCompleted(gestureDescription: GestureDescription) {
                        complete(true)
                    }

                    override fun onCancelled(gestureDescription: GestureDescription) {
                        complete(false)
                    }
                },
                Handler(Looper.getMainLooper()),
            )
        }
    }
}
