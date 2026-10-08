package com.lahfir.agentmobile.driver

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.Assert.assertThrows

class ProvisionProviderTest {

    @Test
    fun predicateAcceptsOnlyRootAndShell() {
        assertTrue(isAuthorizedCaller(0))
        assertTrue(isAuthorizedCaller(2000))
        assertFalse(isAuthorizedCaller(10_000))
        assertFalse(isAuthorizedCaller(10_123))
        assertFalse(isAuthorizedCaller(-1))
    }

    @Test
    fun callRejectsUnsupportedMethod() {
        val provider = ProvisionProvider()
        assertThrows(IllegalArgumentException::class.java) {
            provider.call("bogus", null, null)
        }
    }
}
