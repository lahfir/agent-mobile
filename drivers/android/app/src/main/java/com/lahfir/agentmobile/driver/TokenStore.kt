package com.lahfir.agentmobile.driver

import android.content.Context
import android.content.SharedPreferences
import java.security.SecureRandom
import java.util.Base64

internal data class DriverSession(val token: String, val port: Int)

internal class TokenStore internal constructor(private val prefs: SharedPreferences) {

    constructor(context: Context) : this(
        context.getSharedPreferences(PREFS_FILE, Context.MODE_PRIVATE)
    )

    fun read(): DriverSession? {
        val token = prefs.getString(KEY_TOKEN, null) ?: return null
        val port = prefs.getInt(KEY_PORT, -1)
        return if (port in 1..65535) DriverSession(token, port) else null
    }

    fun replace(session: DriverSession) {
        if (!prefs.edit().putString(KEY_TOKEN, session.token).putInt(KEY_PORT, session.port).commit()) {
            throw IllegalStateException("session persist failed")
        }
    }

    companion object {
        private const val PREFS_FILE = "agent-mobile"
        private const val KEY_TOKEN = "session_token"
        private const val KEY_PORT = "session_port"

        fun generateToken(): String {
            val bytes = ByteArray(32)
            SecureRandom().nextBytes(bytes)
            return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)
        }
    }
}
