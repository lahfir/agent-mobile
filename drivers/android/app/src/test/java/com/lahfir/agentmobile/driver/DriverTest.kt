package com.lahfir.agentmobile.driver

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class DriverTest {

    private fun node(name: String = "", text: String = "") = NodeModel(
        role = "group", name = name, value = "", states = emptyList(),
        availableActions = emptyList(), bounds = LogicalBounds(0.0, 0.0, 10.0, 10.0),
        nativeId = null, children = emptyList(),
        identity = NodeIdentity("android.widget.FrameLayout", "group", "", text, "", RawBounds(0, 0, 10, 10)),
    )

    private fun read(signature: String = "sig", app: String = "com.fake", atNanos: Long = 0) =
        TreeRead(app, node(), true, signature, RawBounds(0, 0, 0, 0), 1.0, atNanos)

    private class FakePlatform(
        var statusData: PlatformStatus = PlatformStatus("com.fake", "emu64a", "17"),
        var trees: List<TreeRead> = emptyList(),
        var fail: DriverException? = null,
        var now: () -> Long = { 0L },
    ) : DriverPlatform {
        var readCalls = 0
        override fun status(): PlatformStatus = statusData
        override fun readTree(): TreeRead {
            fail?.let { throw it }
            val tree = trees.getOrElse(readCalls) { trees.last() }
            readCalls += 1
            return tree.copy(readAtNanos = now())
        }
    }

    private fun driver(
        platform: FakePlatform,
        sleeper: (Long) -> Unit = {},
    ): Driver = Driver(
        platform,
        Settler(
            clock = { 0L },
            sleeper = sleeper,
        ),
    )

    @Test
    fun statusReturnsContractFields() {
        val data = Driver(FakePlatform()).handle("status", JSONObject())
        assertEquals("com.fake", data.getString("app"))
        assertEquals("", data.getString("snapshot_id"))
        assertEquals("emu64a", data.getString("device"))
        assertEquals("17", data.getString("os"))
    }

    @Test
    fun unknownCommandThrowsUnknownCommand() {
        try {
            Driver(FakePlatform()).handle("fizzle", JSONObject())
            fail("expected DriverException")
        } catch (e: DriverException) {
            assertEquals("UNKNOWN_COMMAND", e.code)
        }
    }

    @Test
    fun snapshotReturnsAllContractFields() {
        val platform = FakePlatform(trees = listOf(read()))
        val data = Driver(platform).handle("snapshot", JSONObject())

        assertEquals("com.fake", data.getString("app"))
        assertTrue(data.getString("snapshot_id").matches(Regex("[a-z0-9]{8}")))
        assertEquals(1, data.getInt("ref_count"))
        assertTrue(data.getBoolean("complete"))
        assertTrue(data.has("settled"))
        assertTrue(data.has("reads"))
        assertTrue(data.has("settle_ms"))
        assertTrue(data.has("text"))
        assertTrue(data.has("tree"))
        assertEquals("@${data.getString("snapshot_id")}:e1", data.getJSONObject("tree").getString("ref_id"))
    }

    @Test
    fun statusReflectsLatestSnapshotId() {
        val platform = FakePlatform(trees = listOf(read()))
        val driver = Driver(platform)
        val snap = driver.handle("snapshot", JSONObject())
        val status = driver.handle("status", JSONObject())
        assertEquals(snap.getString("snapshot_id"), status.getString("snapshot_id"))
    }

    @Test
    fun snapshotWithMatchingAppParamSucceeds() {
        val platform = FakePlatform(trees = listOf(read(app = "com.fake")))
        val params = JSONObject().put("app", "com.fake")
        val data = Driver(platform).handle("snapshot", params)
        assertEquals("com.fake", data.getString("app"))
    }

    @Test
    fun snapshotRejectsMismatchedAppParam() {
        val platform = FakePlatform(trees = listOf(read(app = "com.other")))
        val params = JSONObject().put("app", "com.fake")
        try {
            Driver(platform).handle("snapshot", params)
            fail("expected DriverException")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
    }

    @Test
    fun snapshotRejectsMalformedAppParam() {
        val platform = FakePlatform(trees = listOf(read()))
        for (params in listOf(JSONObject().put("app", 7), JSONObject().put("app", ""))) {
            try {
                Driver(platform).handle("snapshot", params)
                fail("expected DriverException for $params")
            } catch (e: DriverException) {
                assertEquals("BAD_REQUEST", e.code)
            }
        }
    }

    @Test
    fun failedSnapshotPreservesPreviousLedger() {
        val platform = FakePlatform(trees = listOf(read()))
        val driver = Driver(platform)
        val first = driver.handle("snapshot", JSONObject())
        platform.fail = DriverException("DRIVER_ERROR", "no active window")
        try {
            driver.handle("snapshot", JSONObject())
            fail("expected DriverException")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        val status = driver.handle("status", JSONObject())
        assertEquals(first.getString("snapshot_id"), status.getString("snapshot_id"))
    }

    @Test
    fun snapshotSettleUsesBaselineDonationOnSecondCall() {
        var now = 200_000_000L
        val platform = FakePlatform(trees = listOf(read(signature = "same", atNanos = 0)), now = { now })
        val driver = Driver(
            platform,
            Settler(clock = { now }, sleeper = { now += it }),
        )
        val first = driver.handle("snapshot", JSONObject())
        assertEquals(2, first.getInt("reads"))
        now += 200_000_000L
        val second = driver.handle("snapshot", JSONObject())
        assertEquals(1, second.getInt("reads"))
        assertTrue(second.getBoolean("settled"))
    }
}
