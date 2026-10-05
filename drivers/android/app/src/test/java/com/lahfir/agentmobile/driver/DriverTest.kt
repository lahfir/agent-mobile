package com.lahfir.agentmobile.driver

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class DriverTest {

    @Test
    fun statusReturnsContractFields() {
        val data = Driver(AgentMobileAccessibilityService()).handle("status", JSONObject())

        assertTrue(data.has("app"))
        assertEquals("", data.getString("snapshot_id"))
        assertTrue(data.has("device"))
        assertTrue(data.has("os"))
    }

    @Test
    fun unknownCommandThrowsUnknownCommand() {
        try {
            Driver(AgentMobileAccessibilityService()).handle("fizzle", JSONObject())
            fail("expected DriverException")
        } catch (e: DriverException) {
            assertEquals("UNKNOWN_COMMAND", e.code)
        }
    }
}
