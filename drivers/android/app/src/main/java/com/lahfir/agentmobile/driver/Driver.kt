package com.lahfir.agentmobile.driver

import android.os.Build
import org.json.JSONObject

internal class Driver(private val service: AgentMobileAccessibilityService) {

    fun handle(command: String, params: JSONObject): JSONObject = when (command) {
        "status" -> status()
        else -> throw DriverException("UNKNOWN_COMMAND", "unknown command: $command")
    }

    private fun status(): JSONObject = JSONObject()
        .put("app", service.rootInActiveWindow?.packageName?.toString() ?: "")
        .put("snapshot_id", "")
        .put("device", Build.MODEL ?: "")
        .put("os", Build.VERSION.RELEASE ?: "")
}
