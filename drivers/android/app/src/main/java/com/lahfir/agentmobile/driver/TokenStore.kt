package com.lahfir.agentmobile.driver

import android.content.Context
import android.content.SharedPreferences
import java.security.SecureRandom
import java.util.Base64

internal class TokenStore internal constructor(private val prefs: SharedPreferences) {

    constructor(context: Context) : this(
        context.getSharedPreferences(PREFS_FILE, Context.MODE_PRIVATE)
    )

    fun read(): String? = prefs.getString(KEY_TOKEN, null)

    fun replace(token: String) {
        if (!prefs.edit().putString(KEY_TOKEN, token).commit()) {
            throw IllegalStateException("token persist failed")
        }
    }

    companion object {
        private const val PREFS_FILE = "agent-mobile"
        private const val KEY_TOKEN = "session_token"

        fun generateToken(): String {
            val bytes = ByteArray(32)
            SecureRandom().nextBytes(bytes)
            return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)
        }
    }
}
