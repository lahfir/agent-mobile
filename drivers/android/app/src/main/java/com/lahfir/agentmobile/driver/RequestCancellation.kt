package com.lahfir.agentmobile.driver

import java.util.concurrent.atomic.AtomicBoolean

internal class RequestCancellation {
    private val cancelled = AtomicBoolean(false)

    fun cancel() {
        cancelled.set(true)
    }

    fun check() {
        if (cancelled.get() || Thread.currentThread().isInterrupted) {
            throw DriverException("DRIVER_ERROR", "request cancelled")
        }
    }

    val isCancelled: Boolean get() = cancelled.get()
}
