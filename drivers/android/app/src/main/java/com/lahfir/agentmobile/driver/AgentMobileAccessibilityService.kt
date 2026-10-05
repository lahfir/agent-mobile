package com.lahfir.agentmobile.driver

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import java.util.concurrent.ExecutionException
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import org.json.JSONObject

class AgentMobileAccessibilityService : AccessibilityService() {

    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }
    private val driver by lazy { Driver(this) }
    private val tokenStore by lazy { TokenStore(this) }
    private var server: HttpServer? = null

    override fun onServiceConnected() {
        serviceInfo = serviceInfo.apply {
            flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS or
                AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS or
                AccessibilityServiceInfo.FLAG_INCLUDE_NOT_IMPORTANT_VIEWS
        }
        activeInstance = this
        tokenStore.read()?.let { startListener(it) }
    }

    internal fun onTokenRotated() {
        mainHandler.post { restartListener() }
    }

    private fun restartListener() {
        server?.close()
        server = null
        tokenStore.read()?.let { startListener(it) }
    }

    private fun startListener(token: String) {
        try {
            server = HttpServer(LISTEN_PORT, token, ::handleRequest).also { it.start() }
        } catch (e: Exception) {
            Log.w(TAG, "listener start failed: ${e.javaClass.simpleName}: ${e.message}")
            server = null
        }
    }

    private fun handleRequest(command: String, params: JSONObject): JSONObject {
        val task = FutureTask { driver.handle(command, params) }
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
            task.cancel(false)
            throw RuntimeException("driver operation timed out")
        } catch (e: InterruptedException) {
            task.cancel(false)
            Thread.currentThread().interrupt()
            throw RuntimeException("driver operation interrupted")
        }
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}

    override fun onInterrupt() {}

    override fun onDestroy() {
        server?.close()
        server = null
        if (activeInstance === this) activeInstance = null
        super.onDestroy()
    }

    companion object {
        private const val LISTEN_PORT = 8770
        private const val MAIN_BRIDGE_TIMEOUT_MS = 10_000L
        private const val TAG = "AgentMobileDriver"

        @Volatile
        internal var activeInstance: AgentMobileAccessibilityService? = null
            private set
    }
}
