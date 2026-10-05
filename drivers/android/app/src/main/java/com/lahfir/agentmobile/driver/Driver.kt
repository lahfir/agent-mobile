package com.lahfir.agentmobile.driver

import android.os.Build
import org.json.JSONObject

internal interface DriverPlatform {
    fun status(): PlatformStatus
    fun readTree(): TreeRead
}

internal data class PlatformStatus(val app: String, val device: String, val os: String)

private class AndroidDriverPlatform(private val service: AgentMobileAccessibilityService) : DriverPlatform {
    private val reader = TreeReader()

    override fun status(): PlatformStatus = service.onMain {
        PlatformStatus(
            app = service.rootInActiveWindow?.let { TreeReader.readPackageName(it) } ?: "",
            device = Build.MODEL ?: "",
            os = Build.VERSION.RELEASE ?: "",
        )
    }

    override fun readTree(): TreeRead = service.onMain {
        val node = service.rootInActiveWindow
            ?: throw DriverException("DRIVER_ERROR", "no active accessibility window; focus a foreground app and retry")
        val density = service.resources.displayMetrics.density.toDouble()
        reader.read(
            TreeReader.createPlatformSource(node),
            readAtNanos = 0L,
            density = density,
        ).copy(readAtNanos = System.nanoTime())
    }
}

internal class Driver internal constructor(
    private val platform: DriverPlatform,
    private val settler: Settler = Settler(),
) {

    constructor(service: AgentMobileAccessibilityService) : this(
        AndroidDriverPlatform(service),
        Settler(),
    )

    internal val ledger = RefLedger()
    private var lastRead: TreeRead? = null

    fun handle(command: String, params: JSONObject): JSONObject = when (command) {
        "status" -> status()
        "snapshot" -> snapshot(params)
        else -> throw DriverException("UNKNOWN_COMMAND", "unknown command: $command")
    }

    private fun status(): JSONObject {
        val current = platform.status()
        return JSONObject()
            .put("app", current.app)
            .put("snapshot_id", ledger.snapshotId)
            .put("device", current.device)
            .put("os", current.os)
    }

    private fun snapshot(params: JSONObject): JSONObject {
        val rawApp = params.opt("app")
        val requestedApp = when {
            rawApp == null || rawApp === JSONObject.NULL -> null
            rawApp is String && rawApp.isNotEmpty() -> rawApp
            else -> throw DriverException("BAD_REQUEST", "app must be a nonempty string")
        }
        val result = settler.settle(lastRead) { platform.readTree() }
        if (requestedApp != null && requestedApp != result.read.app) {
            throw DriverException("BAD_REQUEST", "app $requestedApp is not the foreground package ${result.read.app}")
        }
        val out = ledger.mint(result.read, result.settled, result.reads, result.settleMs)
        lastRead = result.read
        return out
    }
}
