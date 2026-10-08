package com.lahfir.agentmobile.driver

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import java.util.concurrent.ExecutionException
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import org.json.JSONObject

class AgentMobileAccessibilityService : AccessibilityService() {

    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }
    private val driver by lazy { Driver(this) }
    private val tokenStore by lazy { TokenStore(this) }
    internal val screenshotExecutor: ExecutorService = Executors.newSingleThreadExecutor { r ->
        Thread(r, "agent-mobile-screenshot").apply { isDaemon = true }
    }
    private val serverLock = Any()
    private var server: HttpServer? = null
    private var destroyed = false

    override fun onServiceConnected() {
        serviceInfo = serviceInfo.apply {
            flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS or
                AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS or
                AccessibilityServiceInfo.FLAG_INCLUDE_NOT_IMPORTANT_VIEWS
        }
        synchronized(serverLock) {
            if (!destroyed) {
                activeInstance = this
                tokenStore.read()?.let { startListener(it) }
            }
        }
    }

    internal fun rotateSession(token: String): Int = synchronized(serverLock) {
        check(!destroyed) { "accessibility service destroyed" }
        val next = HttpServer(0, token, ::handleRequest)
        next.start()
        val session = DriverSession(token, next.localPort)
        try {
            tokenStore.replace(session)
        } catch (e: Exception) {
            next.close()
            throw e
        }
        val previous = server
        server = next
        previous?.close()
        session.port
    }

    private fun startListener(session: DriverSession) {
        try {
            server = HttpServer(session.port, session.token, ::handleRequest).also { it.start() }
        } catch (e: Exception) {
            Log.w(TAG, "listener start failed: ${e.javaClass.simpleName}: ${e.message}")
            server = null
        }
    }

    internal fun <T> onMain(
        cancellation: RequestCancellation = RequestCancellation(),
        block: () -> T,
    ): T {
        cancellation.check()
        if (Looper.myLooper() == Looper.getMainLooper()) {
            val result = block()
            cancellation.check()
            return result
        }
        val task = FutureTask(java.util.concurrent.Callable {
            cancellation.check()
            val result = block()
            cancellation.check()
            result
        })
        if (!mainHandler.post(task)) {
            throw RuntimeException("driver main thread unavailable")
        }
        return try {
            task.get(MAIN_BRIDGE_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        } catch (e: ExecutionException) {
            when (val cause = e.cause) {
                is DriverException -> throw cause
                null -> throw RuntimeException("driver operation failed")
                else -> throw cause
            }
        } catch (e: TimeoutException) {
            cancellation.cancel()
            task.cancel(false)
            throw DriverException("DRIVER_ERROR", "driver operation timed out")
        } catch (e: InterruptedException) {
            cancellation.cancel()
            task.cancel(false)
            Thread.currentThread().interrupt()
            throw DriverException("DRIVER_ERROR", "driver operation interrupted")
        }
    }

    private fun handleRequest(command: String, params: JSONObject, cancellation: RequestCancellation): JSONObject =
        driver.handle(command, params, cancellation)

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}

    override fun onInterrupt() {}

    override fun onDestroy() {
        synchronized(serverLock) {
            destroyed = true
            if (activeInstance === this) activeInstance = null
            server?.close()
            server = null
        }
        screenshotExecutor.shutdownNow()
        super.onDestroy()
    }

    companion object {
        private const val MAIN_BRIDGE_TIMEOUT_MS = 20_000L
        private const val TAG = "AgentMobileDriver"

        @Volatile
        internal var activeInstance: AgentMobileAccessibilityService? = null
            private set
    }
}
