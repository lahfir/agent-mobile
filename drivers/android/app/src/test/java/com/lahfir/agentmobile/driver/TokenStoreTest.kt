package com.lahfir.agentmobile.driver

import android.content.SharedPreferences
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TokenStoreTest {

    private class FakeEditor(private val backing: MutableMap<String, Any?>) : SharedPreferences.Editor {
        private val pending = mutableMapOf<String, Any?>()
        private var cleared = false

        override fun putString(key: String?, value: String?) = apply { pending[requireNotNull(key)] = value }
        override fun putStringSet(key: String?, value: MutableSet<String>?) = apply { pending[requireNotNull(key)] = value }
        override fun putInt(key: String?, value: Int) = apply { pending[requireNotNull(key)] = value }
        override fun putLong(key: String?, value: Long) = apply { pending[requireNotNull(key)] = value }
        override fun putFloat(key: String?, value: Float) = apply { pending[requireNotNull(key)] = value }
        override fun putBoolean(key: String?, value: Boolean) = apply { pending[requireNotNull(key)] = value }
        override fun remove(key: String?) = apply { pending[requireNotNull(key)] = REMOVED }
        override fun clear() = apply { cleared = true }
        override fun commit(): Boolean {
            if (cleared) backing.clear()
            for ((k, v) in pending) {
                if (v === REMOVED) backing.remove(k) else backing[k] = v
            }
            return true
        }
        override fun apply() {
            commit()
        }

        private companion object {
            val REMOVED = Any()
        }
    }

    private class FakePrefs : SharedPreferences {
        val backing = mutableMapOf<String, Any?>()

        override fun getAll(): MutableMap<String, Any?> = backing
        override fun getString(key: String?, defValue: String?): String? = backing[key] as? String ?: defValue
        override fun getStringSet(key: String?, defValues: MutableSet<String>?): MutableSet<String>? =
            @Suppress("UNCHECKED_CAST") (backing[key] as? MutableSet<String>) ?: defValues
        override fun getInt(key: String?, defValue: Int): Int = backing[key] as? Int ?: defValue
        override fun getLong(key: String?, defValue: Long): Long = backing[key] as? Long ?: defValue
        override fun getFloat(key: String?, defValue: Float): Float = backing[key] as? Float ?: defValue
        override fun getBoolean(key: String?, defValue: Boolean): Boolean = backing[key] as? Boolean ?: defValue
        override fun contains(key: String?): Boolean = backing.containsKey(key)
        override fun edit(): SharedPreferences.Editor = FakeEditor(backing)
        override fun registerOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) {}
        override fun unregisterOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) {}
    }

    @Test
    fun generatedTokenIsUrlSafeUnpaddedAndHighEntropy() {
        val token = TokenStore.generateToken()
        assertEquals(43, token.length)
        assertTrue(token.all { it.isLetterOrDigit() || it == '-' || it == '_' })
        assertFalse(token.contains('='))
    }

    @Test
    fun generatedTokensAreUnique() {
        val tokens = (1..64).map { TokenStore.generateToken() }.toSet()
        assertEquals(64, tokens.size)
    }

    @Test
    fun readReturnsNullWhenUnset() {
        assertNull(TokenStore(FakePrefs()).read())
    }

    @Test
    fun replacePersistsSessionSynchronously() {
        val store = TokenStore(FakePrefs())
        store.replace(DriverSession("token-one", 4321))
        assertEquals(DriverSession("token-one", 4321), store.read())
        store.replace(DriverSession("token-two", 9876))
        assertEquals(DriverSession("token-two", 9876), store.read())
    }

    @Test
    fun readReturnsNullWhenPortIsAbsentOrInvalid() {
        val prefs = FakePrefs()
        prefs.edit().putString("session_token", "token-one").commit()
        assertNull(TokenStore(prefs).read())
        prefs.edit().putInt("session_port", 0).commit()
        assertNull(TokenStore(prefs).read())
        prefs.edit().putInt("session_port", 70000).commit()
        assertNull(TokenStore(prefs).read())
        prefs.edit().putInt("session_port", 8770).commit()
        assertEquals(DriverSession("token-one", 8770), TokenStore(prefs).read())
    }
}
