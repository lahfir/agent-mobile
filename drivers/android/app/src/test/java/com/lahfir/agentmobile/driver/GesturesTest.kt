package com.lahfir.agentmobile.driver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import kotlin.math.abs

class GesturesTest {

    private fun node() = NodeModel(
        role = "group", name = "", value = "", states = emptyList(),
        availableActions = emptyList(), bounds = LogicalBounds(0.0, 0.0, 0.0, 0.0),
        nativeId = null, children = emptyList(),
        identity = NodeIdentity("", "", "", "", "", RawBounds(0, 0, 0, 0)),
    )

    private fun read(origin: RawBounds, density: Double) =
        TreeRead("com.fake", node(), true, "sig", origin, density, 0L)

    private fun stroke(spec: GestureSpec, i: Int) = spec.strokes[i]

    @Test
    fun logicalToRawUsesOriginAndDensity() {
        val read = read(RawBounds(10, 20, 210, 220), 2.0)
        val raw = Gestures.toRaw(RawPoint(5f, 10f), read)
        assertEquals(20f, raw.x, 0.001f)
        assertEquals(40f, raw.y, 0.001f)
    }

    @Test
    fun refCenterEqualsLogicalCenterUnderDensityAndInset() {
        val read = read(RawBounds(10, 20, 410, 620), 2.0)
        val rawBounds = RawBounds(30, 60, 130, 160)
        val logical = LogicalBounds(
            x = (30 - 10) / 2.0, y = (60 - 20) / 2.0,
            width = 100 / 2.0, height = 100 / 2.0,
        )
        val viaLogical = Gestures.toRaw(
            RawPoint((logical.x + logical.width / 2).toFloat(), (logical.y + logical.height / 2).toFloat()),
            read,
        )
        val viaRaw = Gestures.rawCenter(rawBounds)
        assertEquals(viaRaw.x, viaLogical.x, 0.01f)
        assertEquals(viaRaw.y, viaLogical.y, 0.01f)
    }

    @Test
    fun tapStrokeIs50ms() {
        val spec = Gestures.tap(RawPoint(10f, 20f))
        assertEquals(1, spec.strokes.size)
        assertEquals(0, stroke(spec, 0).startMs)
        assertEquals(50, stroke(spec, 0).durationMs)
        assertEquals(listOf(RawPoint(10f, 20f)), stroke(spec, 0).points)
    }

    @Test
    fun doubleTapIsTwoStrokesSeparatedBy100ms() {
        val spec = Gestures.doubleTap(RawPoint(5f, 5f))
        assertEquals(2, spec.strokes.size)
        assertEquals(0, stroke(spec, 0).startMs)
        assertEquals(50, stroke(spec, 0).durationMs)
        assertEquals(150, stroke(spec, 1).startMs)
        assertEquals(50, stroke(spec, 1).durationMs)
    }

    @Test
    fun holdConvertsSecondsToMillis() {
        val spec = Gestures.hold(RawPoint(7f, 7f), 1_500)
        assertEquals(1_500, stroke(spec, 0).durationMs)
    }

    @Test
    fun swipeDirectionsTravelAlongAxis() {
        val bounds = RawBounds(0, 0, 200, 400)
        for ((dir, dxSign, dySign) in listOf(
            Triple("up", 0, -1), Triple("down", 0, 1),
            Triple("left", -1, 0), Triple("right", 1, 0),
        )) {
            val spec = Gestures.swipe(bounds, dir, 0.5)
            val pts = stroke(spec, 0).points
            val first = pts.first(); val last = pts.last()
            if (dxSign != 0) {
                assertTrue("$dir must travel in x", (last.x - first.x) * dxSign > 0)
                assertEquals(first.y, last.y, 0.001f)
            } else {
                assertEquals(first.x, last.x, 0.001f)
                assertTrue("$dir must travel in y", (last.y - first.y) * dySign > 0)
            }
            assertEquals(200, stroke(spec, 0).durationMs)
            val span = if (dxSign != 0) abs(last.x - first.x) else abs(last.y - first.y)
            val extent = (if (dxSign != 0) bounds.width else bounds.height) * 0.5f
            assertEquals(extent, span, 0.5f)
        }
    }

    @Test
    fun pinchUsesTwoSymmetricStrokesOnLongerAxisInsideBounds() {
        val bounds = RawBounds(0, 0, 400, 200)
        val zoomIn = Gestures.pinch(bounds, 2.0, 0.0)
        assertEquals(2, zoomIn.strokes.size)
        assertEquals(stroke(zoomIn, 0).startMs, stroke(zoomIn, 1).startMs)
        val a0 = stroke(zoomIn, 0).points
        val b0 = stroke(zoomIn, 1).points
        val startGap = abs(a0.first().x - b0.first().x)
        val endGap = abs(a0.last().x - b0.last().x)
        assertTrue("scale>1 must spread apart", endGap > startGap)
        zoomIn.strokes.flatMap { it.points }.forEach { p ->
            assertTrue("x $p inside", p.x > bounds.left && p.x < bounds.right)
            assertTrue("y $p inside", p.y > bounds.top && p.y < bounds.bottom)
        }
        val zoomOut = Gestures.pinch(bounds, 0.5, 0.0)
        val a1 = stroke(zoomOut, 0).points
        val b1 = stroke(zoomOut, 1).points
        assertTrue("scale<1 must converge", abs(a1.last().x - b1.last().x) < abs(a1.first().x - b1.first().x))
    }

    @Test
    fun pinchVelocityBoundsDuration() {
        val bounds = RawBounds(0, 0, 400, 200)
        val fast = Gestures.pinch(bounds, 2.0, 10_000.0)
        val slow = Gestures.pinch(bounds, 2.0, 0.001)
        assertTrue(stroke(fast, 0).durationMs >= 100)
        assertTrue(stroke(slow, 0).durationMs <= 1_000)
        assertTrue(stroke(Gestures.pinch(bounds, 2.0, 0.0), 0).durationMs in 100..1_000)
    }

    @Test
    fun twofingerHasTwoSimultaneousStationaryStrokes() {
        val bounds = RawBounds(0, 0, 200, 400)
        val spec = Gestures.twoFinger(bounds)
        assertEquals(2, spec.strokes.size)
        assertEquals(stroke(spec, 0).startMs, stroke(spec, 1).startMs)
        assertEquals(1, stroke(spec, 0).points.size)
        assertEquals(1, stroke(spec, 1).points.size)
        val p0 = stroke(spec, 0).points[0]; val p1 = stroke(spec, 1).points[0]
        assertTrue(abs(p0.y - p1.y) > 0)
        spec.strokes.flatMap { it.points }.forEach { p ->
            assertTrue(p.y > bounds.top && p.y < bounds.bottom)
        }
    }

    @Test
    fun pinchScaleControlsOffsetMagnitude() {
        val bounds = RawBounds(0, 0, 400, 200)
        fun gap(spec: GestureSpec, first: Boolean): Float {
            val a = stroke(spec, 0).points.let { if (first) it.first() else it.last() }
            val b = stroke(spec, 1).points.let { if (first) it.first() else it.last() }
            return abs(a.x - b.x)
        }
        val out4 = Gestures.pinch(bounds, 4.0, 0.0)
        val out2 = Gestures.pinch(bounds, 2.0, 0.0)
        assertTrue("scale 4 must start closer than scale 2", gap(out4, true) < gap(out2, true))
        val inQuarter = Gestures.pinch(bounds, 0.25, 0.0)
        val inHalf = Gestures.pinch(bounds, 0.5, 0.0)
        assertTrue("scale .25 must end closer than .5", gap(inQuarter, false) < gap(inHalf, false))
    }

    @Test
    fun degenerateBoundsRejectGestures() {
        try {
            Gestures.rawCenter(RawBounds(0, 0, 0, 0))
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        try {
            Gestures.pinch(RawBounds(0, 0, 0, 0), 2.0, 0.0)
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        try {
            Gestures.swipe(RawBounds(0, 0, 0, 100), "up", 0.5)
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
    }
}
