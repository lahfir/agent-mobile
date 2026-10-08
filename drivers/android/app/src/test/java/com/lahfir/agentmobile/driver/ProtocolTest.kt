package com.lahfir.agentmobile.driver

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtocolTest {

    @Test
    fun successEnvelopeCarriesContractFields() {
        val data = JSONObject().put("app", "com.example")
        val envelope = Protocol.success("status", 12, data)

        assertEquals("1", envelope.getString("version"))
        assertTrue(envelope.getBoolean("ok"))
        assertEquals("status", envelope.getString("command"))
        assertEquals(12L, envelope.getLong("elapsed_ms"))
        assertEquals("com.example", envelope.getJSONObject("data").getString("app"))
    }

    @Test
    fun failureEnvelopeCarriesCommandAndElapsed() {
        val envelope = Protocol.failure("tap", 34, "STALE_REF", "ref no longer matches")

        assertEquals("1", envelope.getString("version"))
        assertFalse(envelope.getBoolean("ok"))
        assertEquals("tap", envelope.getString("command"))
        assertEquals(34L, envelope.getLong("elapsed_ms"))
        val error = envelope.getJSONObject("error")
        assertEquals("STALE_REF", error.getString("code"))
        assertEquals("ref no longer matches", error.getString("message"))
    }

    @Test
    fun failureEnvelopeOmitsCommandAndElapsedWhenNull() {
        val envelope = Protocol.failure(null, null, "UNAUTHORIZED", "bearer required")

        assertEquals("1", envelope.getString("version"))
        assertFalse(envelope.getBoolean("ok"))
        assertFalse(envelope.has("command"))
        assertFalse(envelope.has("elapsed_ms"))
        assertEquals("UNAUTHORIZED", envelope.getJSONObject("error").getString("code"))
    }

    @Test
    fun driverExceptionCarriesCodeAndMessage() {
        val e = DriverException("UNKNOWN_COMMAND", "unknown command: frobnicate")
        assertEquals("UNKNOWN_COMMAND", e.code)
        assertEquals("unknown command: frobnicate", e.message)
    }
}
