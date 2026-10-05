package com.lahfir.agentmobile.driver

import org.json.JSONObject

internal class DriverException(val code: String, message: String) : RuntimeException(message)

internal object Protocol {
    const val VERSION = "1"

    fun statusForCode(code: String): Int = when (code) {
        "UNAUTHORIZED" -> 401
        "BAD_REQUEST", "UNKNOWN_COMMAND", "STALE_REF", "AMBIGUOUS_TARGET" -> 409
        else -> 500
    }

    fun success(command: String, elapsedMs: Long, data: JSONObject): JSONObject =
        JSONObject()
            .put("version", VERSION)
            .put("ok", true)
            .put("command", command)
            .put("elapsed_ms", elapsedMs)
            .put("data", data)

    fun failure(command: String?, elapsedMs: Long?, code: String, message: String): JSONObject {
        val envelope = JSONObject()
            .put("version", VERSION)
            .put("ok", false)
        if (command != null) envelope.put("command", command)
        if (elapsedMs != null) envelope.put("elapsed_ms", elapsedMs)
        envelope.put("error", JSONObject().put("code", code).put("message", message))
        return envelope
    }
}
