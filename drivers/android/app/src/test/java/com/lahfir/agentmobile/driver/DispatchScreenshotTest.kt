package com.lahfir.agentmobile.driver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

class DispatchScreenshotTest {

    private fun spec(points: Int = 1, strokes: Int = 1, durationMs: Long = 50): GestureSpec =
        GestureSpec(
            (0 until strokes).map {
                StrokeSpec((0 until points).map { p -> RawPoint(p.toFloat(), p.toFloat()) }, 0, durationMs)
            },
        )

    private fun dispatcher(
        submitted: AtomicInteger = AtomicInteger(),
        timeoutMs: Long = 5_000,
        onSubmit: (GestureSpec, (Boolean) -> Unit) -> Boolean = { _, cb -> cb(true); true },
    ): GestureDispatcher = GestureDispatcher(
        GestureSubmission { spec, complete -> onSubmit(spec, complete) },
        timeoutMarginMs = timeoutMs,
    )

    @Test
    fun acceptedDispatchCompletes() {
        val submitted = AtomicInteger()
        dispatcher(submitted) { spec, cb ->
            submitted.incrementAndGet()
            cb(true)
            true
        }.dispatch(spec())
        assertEquals(1, submitted.get())
    }

    @Test
    fun refusedSubmitMapsDriverError() {
        try {
            dispatcher { _, _ -> false }.dispatch(spec())
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
    }

    @Test
    fun cancelledGestureMapsDriverError() {
        try {
            dispatcher { _, cb -> cb(false); true }.dispatch(spec())
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
    }

    @Test
    fun missingCallbackTimesOutBoundedly() {
        val start = System.currentTimeMillis()
        try {
            dispatcher(timeoutMs = 1) { _, _ -> true }.dispatch(spec(durationMs = 1))
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
            assertTrue("timeout must stay bounded", System.currentTimeMillis() - start < 5_000)
        }
    }

    @Test
    fun interruptedDispatchWaitPreservesInterrupt() {
        Thread.currentThread().interrupt()
        try {
            dispatcher(timeoutMs = 60_000) { _, _ -> true }.dispatch(spec())
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertTrue(Thread.interrupted())
    }

    @Test
    fun invalidSpecsRejectedBeforeDispatch() {
        val submitted = AtomicInteger()
        val empty = GestureSpec(emptyList())
        try {
            dispatcher(submitted).dispatch(empty)
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
        val nan = GestureSpec(listOf(StrokeSpec(listOf(RawPoint(Float.NaN, 0f)), 0, 50)))
        try {
            dispatcher(submitted).dispatch(nan)
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
        val tooMany = spec(strokes = 99)
        try {
            dispatcher(submitted).dispatch(tooMany)
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
        assertEquals(0, submitted.get())
    }
}

class ScreenshotsTest {

    private val png = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3)

    @Test
    fun successReturnsPngBytes() {
        val shots = Screenshots(
            ScreenshotBackend { cb -> cb(Result.success(png)) },
            timeoutMs = 100,
        )
        assertTrue(shots.capture().contentEquals(png))
    }

    @Test
    fun backendErrorMapsDriverError() {
        val shots = Screenshots(
            ScreenshotBackend { cb -> cb(Result.failure(DriverException("DRIVER_ERROR", "secure window"))) },
            timeoutMs = 100,
        )
        try {
            shots.capture()
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
            assertTrue(e.message!!.contains("secure window"))
        }
    }

    @Test
    fun missingCallbackTimesOut() {
        val shots = Screenshots(ScreenshotBackend { }, timeoutMs = 1)
        try {
            shots.capture()
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
    }

    @Test
    fun interruptedScreenshotWaitPreservesInterrupt() {
        val shots = Screenshots(ScreenshotBackend { }, timeoutMs = 60_000)
        Thread.currentThread().interrupt()
        try {
            shots.capture()
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertTrue(Thread.interrupted())
    }

    @Test
    fun errorCodeMappingIsStable() {
        assertTrue(Screenshots.errorMessage(android.accessibilityservice.AccessibilityService.ERROR_TAKE_SCREENSHOT_SECURE_WINDOW).contains("secure window"))
        assertTrue(Screenshots.errorMessage(android.accessibilityservice.AccessibilityService.ERROR_TAKE_SCREENSHOT_INVALID_WINDOW).contains("window"))
        assertTrue(Screenshots.errorMessage(android.accessibilityservice.AccessibilityService.ERROR_TAKE_SCREENSHOT_NO_ACCESSIBILITY_ACCESS).isNotEmpty())
        assertTrue(Screenshots.errorMessage(android.accessibilityservice.AccessibilityService.ERROR_TAKE_SCREENSHOT_INTERNAL_ERROR).isNotEmpty())
        assertTrue(Screenshots.errorMessage(999).isNotEmpty())
    }

    @Test
    fun firstCallbackWinsLateResultsCannotOverwrite() {
        val shots = Screenshots(
            ScreenshotBackend { cb ->
                cb(Result.success(png))
                cb(Result.failure(DriverException("DRIVER_ERROR", "late failure")))
            },
            timeoutMs = 100,
        )
        assertTrue(shots.capture().contentEquals(png))
    }
}
