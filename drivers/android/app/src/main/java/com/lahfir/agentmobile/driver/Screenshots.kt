package com.lahfir.agentmobile.driver

import android.accessibilityservice.AccessibilityService
import android.graphics.Bitmap
import android.view.Display
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

internal fun interface ScreenshotBackend {
    fun capture(complete: (Result<ByteArray>) -> Unit)
}

internal class Screenshots(
    private val backend: ScreenshotBackend,
    private val timeoutMs: Long = 5_000,
) {

    fun capture(): ByteArray {
        val latch = CountDownLatch(1)
        val result = AtomicReference<Result<ByteArray>>()
        val delivered = java.util.concurrent.atomic.AtomicBoolean()
        try {
            backend.capture { r ->
                if (delivered.compareAndSet(false, true)) {
                    result.set(r)
                    latch.countDown()
                }
            }
        } catch (e: Exception) {
            throw DriverException("DRIVER_ERROR", "screenshot failed: ${e.javaClass.simpleName}")
        }
        val arrived = try {
            latch.await(timeoutMs, TimeUnit.MILLISECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            throw DriverException("DRIVER_ERROR", "screenshot wait interrupted")
        }
        if (!arrived) {
            throw DriverException("DRIVER_ERROR", "screenshot timed out")
        }
        return result.get().getOrThrow()
    }

    companion object {
        internal fun errorMessage(code: Int): String = when (code) {
            AccessibilityService.ERROR_TAKE_SCREENSHOT_SECURE_WINDOW ->
                "screenshot refused: secure window"
            AccessibilityService.ERROR_TAKE_SCREENSHOT_NO_ACCESSIBILITY_ACCESS ->
                "screenshot refused: no accessibility access"
            AccessibilityService.ERROR_TAKE_SCREENSHOT_INVALID_DISPLAY ->
                "screenshot failed: invalid display"
            AccessibilityService.ERROR_TAKE_SCREENSHOT_INVALID_WINDOW ->
                "screenshot failed: invalid window"
            AccessibilityService.ERROR_TAKE_SCREENSHOT_INTERVAL_TIME_SHORT ->
                "screenshot refused: interval too short"
            AccessibilityService.ERROR_TAKE_SCREENSHOT_INTERNAL_ERROR ->
                "screenshot failed: internal error"
            else -> "screenshot failed: code $code"
        }
    }
}

internal class AndroidScreenshotBackend(
    private val service: AgentMobileAccessibilityService,
) : ScreenshotBackend {

    override fun capture(complete: (Result<ByteArray>) -> Unit) {
        service.onMain {
            service.takeScreenshot(
                Display.DEFAULT_DISPLAY,
                service.screenshotExecutor,
                object : AccessibilityService.TakeScreenshotCallback {
                    override fun onSuccess(result: AccessibilityService.ScreenshotResult) {
                        val buffer = result.hardwareBuffer
                        try {
                            val hardware = Bitmap.wrapHardwareBuffer(buffer, result.colorSpace)
                            try {
                                val copy = hardware?.copy(Bitmap.Config.ARGB_8888, false)
                                try {
                                    val out = ByteArrayOutputStream()
                                    val ok = copy != null && copy.compress(Bitmap.CompressFormat.PNG, 100, out)
                                    if (ok) {
                                        complete(Result.success(out.toByteArray()))
                                    } else {
                                        complete(Result.failure(DriverException("DRIVER_ERROR", "screenshot produced no pixels")))
                                    }
                                } finally {
                                    copy?.recycle()
                                }
                            } finally {
                                hardware?.recycle()
                            }
                        } catch (e: Exception) {
                            complete(Result.failure(DriverException("DRIVER_ERROR", "screenshot failed: ${e.javaClass.simpleName}")))
                        } finally {
                            buffer.close()
                        }
                    }

                    override fun onFailure(errorCode: Int) {
                        complete(Result.failure(DriverException("DRIVER_ERROR", Screenshots.errorMessage(errorCode))))
                    }
                },
            )
        }
    }
}
