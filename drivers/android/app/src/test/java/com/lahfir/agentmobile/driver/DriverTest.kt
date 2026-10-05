package com.lahfir.agentmobile.driver

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

private fun fixtureNode(
    name: String = "",
    text: String = "",
    bounds: RawBounds = RawBounds(0, 0, 100, 100),
    states: List<String> = emptyList(),
    actions: List<String> = emptyList(),
    className: String = "android.widget.FrameLayout",
    role: String = "group",
    children: List<NodeModel> = emptyList(),
) = NodeModel(
    role = role, name = name, value = "", states = states,
    availableActions = actions, bounds = LogicalBounds(0.0, 0.0, bounds.width.toDouble(), bounds.height.toDouble()),
    nativeId = null, children = children,
    identity = NodeIdentity(className, role, "", text, "", bounds),
)

private fun fixtureRead(
    signature: String = "sig",
    app: String = "com.fake",
    root: NodeModel = fixtureNode(bounds = RawBounds(0, 0, 400, 800)),
) = TreeRead(app, root, true, signature, root.identity.rawBounds, 1.0, 0L)

class DriverTest {

    private fun node(name: String = "", text: String = "") = fixtureNode(name = name, text = text)

    private fun read(signature: String = "sig", app: String = "com.fake", atNanos: Long = 0) =
        fixtureRead(signature, app).copy(readAtNanos = atNanos)

    private class FakePlatform(
        var statusData: PlatformStatus = PlatformStatus("com.fake", "emu64a", "17"),
        var trees: List<TreeRead> = emptyList(),
        var fail: DriverException? = null,
        var now: () -> Long = System::nanoTime,
        var actionAccepted: Boolean = true,
        var actionResultNode: NodeModel? = null,
        var globalResult: Boolean = true,
        var png: ByteArray = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47),
        var focusedAccepted: Boolean = true,
    ) : DriverPlatform {
        var readCalls = 0
        val nodeActions = mutableListOf<NodeAction>()
        val gestures = mutableListOf<GestureSpec>()
        val globals = mutableListOf<GlobalAction>()
        var focusedCalls = 0
        override fun status(): PlatformStatus = statusData
        override fun readTree(): TreeRead {
            fail?.let { throw it }
            val tree = trees.getOrElse(readCalls) { trees.last() }
            readCalls += 1
            return tree.copy(readAtNanos = now())
        }
        override fun performNodeAction(identity: NodeIdentity, action: NodeAction): NodeActionResult {
            nodeActions += action
            val tree = trees.firstOrNull() ?: fixtureRead().copy(readAtNanos = now())
            return NodeActionResult(
                read = tree,
                node = actionResultNode ?: tree.root,
                accepted = actionAccepted,
            )
        }
        override fun appendToFocused(text: String): NodeActionResult {
            focusedCalls += 1
            return NodeActionResult(trees.first(), trees.first().root, focusedAccepted)
        }
        override fun dispatchGesture(spec: GestureSpec) {
            gestures += spec
        }
        override fun performGlobal(action: GlobalAction): Boolean {
            globals += action
            return globalResult
        }
        override fun screenshot(): ByteArray = png
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

    private fun mintedDriver(platform: FakePlatform): Pair<Driver, String> {
        val driver = Driver(platform)
        val snap = driver.handle("snapshot", JSONObject())
        return driver to snap.getString("snapshot_id")
    }

    @Test
    fun tapRefUsesSemanticClickAndRotatesSnapshot() {
        val platform = FakePlatform(trees = listOf(read()))
        val (driver, firstId) = mintedDriver(platform)
        val out = driver.handle("tap", JSONObject().put("ref", "@$firstId:e1"))
        assertTrue(platform.nodeActions.single() is NodeAction.Click)
        assertTrue(platform.gestures.isEmpty())
        val newId = out.getString("snapshot_id")
        assertTrue(newId != firstId && newId.matches(Regex("[a-z0-9]{8}")))
        try {
            driver.handle("tap", JSONObject().put("ref", "@$firstId:e1"))
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
        assertEquals("old ref must fail before another action", 1, platform.nodeActions.size)
    }

    @Test
    fun tapRefFallsBackToGestureWhenSemanticRefused() {
        val platform = FakePlatform(trees = listOf(read()), actionAccepted = false)
        val (driver, firstId) = mintedDriver(platform)
        driver.handle("tap", JSONObject().put("ref", "@$firstId:e1"))
        assertEquals(1, platform.nodeActions.size)
        assertEquals(1, platform.gestures.size)
        assertEquals(50, platform.gestures[0].strokes[0].durationMs)
    }

    @Test
    fun tapPointDispatchesGestureAtRawCoordinates() {
        val platform = FakePlatform(trees = listOf(read()))
        val driver = Driver(platform)
        driver.handle("tap", JSONObject().put("x", 10).put("y", 20))
        assertEquals(1, platform.gestures.size)
        val p = platform.gestures[0].strokes[0].points[0]
        assertEquals(10f, p.x, 0.01f)
        assertEquals(20f, p.y, 0.01f)
        assertTrue(platform.nodeActions.isEmpty())
    }

    @Test
    fun tapBodyValidation() {
        val driver = Driver(FakePlatform(trees = listOf(read())))
        try {
            driver.handle("tap", JSONObject().put("x", 5))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
            assertEquals("x and y must be sent together", e.message)
        }
        try {
            driver.handle("tap", JSONObject())
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
            assertEquals("ref required", e.message)
        }
        try {
            driver.handle("tap", JSONObject().put("x", "NaN").put("y", 1))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
        try {
            driver.handle("tap", JSONObject().put("x", 99999).put("y", 20))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
    }

    @Test
    fun doubletapAndHoldDispatchGestures() {
        val platform = FakePlatform(trees = listOf(read()))
        val driver = Driver(platform)
        driver.handle("doubletap", JSONObject().put("x", 10).put("y", 10))
        assertEquals(2, platform.gestures[0].strokes.size)
        driver.handle("hold", JSONObject().put("x", 10).put("y", 10).put("duration", 0.5))
        assertEquals(500, platform.gestures[1].strokes[0].durationMs)
        driver.handle("hold", JSONObject().put("x", 10).put("y", 10))
        assertEquals(1000, platform.gestures[2].strokes[0].durationMs)
    }

    @Test
    fun holdDurationValidation() {
        val driver = Driver(FakePlatform(trees = listOf(read())))
        for (bad in listOf(JSONObject().put("duration", "slow"), JSONObject().put("duration", -1.0), JSONObject().put("duration", 11.0))) {
            bad.put("x", 5).put("y", 5)
            try {
                driver.handle("hold", bad)
                fail("expected BAD_REQUEST for $bad")
            } catch (e: DriverException) {
                assertEquals("BAD_REQUEST", e.code)
            }
        }
    }

    @Test
    fun pinchValidatesAndDispatchesTwoStrokes() {
        val platform = FakePlatform(trees = listOf(read()))
        val (driver, id) = mintedDriver(platform)
        for (bad in listOf(JSONObject(), JSONObject().put("scale", "x"), JSONObject().put("scale", 1.0), JSONObject().put("scale", -2.0))) {
            bad.put("ref", "@$id:e1")
            try {
                driver.handle("pinch", bad)
                fail("expected BAD_REQUEST for $bad")
            } catch (e: DriverException) {
                assertEquals("BAD_REQUEST", e.code)
                assertEquals("scale positive, finite, |scale-1| >= 0.01", e.message)
            }
        }
        try {
            driver.handle("pinch", JSONObject().put("ref", "@$id:e1").put("scale", 2.0).put("velocity", "fast"))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("velocity finite", e.message)
        }
        driver.handle("pinch", JSONObject().put("ref", "@$id:e1").put("scale", 2.0))
        assertEquals(2, platform.gestures.last().strokes.size)
    }

    @Test
    fun twofingerRequiresRef() {
        val driver = Driver(FakePlatform(trees = listOf(read())))
        try {
            driver.handle("twofinger", JSONObject())
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
    }

    @Test
    fun swipeDirectionAndFallbacks() {
        val platform = FakePlatform(trees = listOf(read()), actionAccepted = true)
        val (driver, id) = mintedDriver(platform)
        try {
            driver.handle("swipe", JSONObject().put("direction", "diagonal"))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("direction up|down|left|right", e.message)
        }
        driver.handle("swipe", JSONObject().put("ref", "@$id:e1").put("direction", "down"))
        assertTrue(platform.nodeActions.any { it is NodeAction.Scroll && it.direction == "down" })
        assertTrue(platform.gestures.isEmpty())
        platform.actionAccepted = false
        val (_, id2) = driver.handle("snapshot", JSONObject()).getString("snapshot_id").let { Unit to it }
        driver.handle("swipe", JSONObject().put("ref", "@$id2:e1").put("direction", "up"))
        assertEquals(1, platform.gestures.size)
        driver.handle("swipe", JSONObject().put("direction", "up"))
        assertEquals(2, platform.gestures.size)
    }

    @Test
    fun typeUsesRefAppendOrFocusedFallback() {
        val platform = FakePlatform(trees = listOf(read()))
        val (driver, id) = mintedDriver(platform)
        driver.handle("type", JSONObject().put("ref", "@$id:e1").put("text", "hello"))
        assertTrue(platform.nodeActions.single() is NodeAction.AppendText)
        platform.actionAccepted = false
        val (_, id2) = Unit to driver.handle("snapshot", JSONObject()).getString("snapshot_id")
        try {
            driver.handle("type", JSONObject().put("ref", "@$id2:e1").put("text", "x"))
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        driver.handle("type", JSONObject().put("text", "focused text"))
        assertEquals(1, platform.focusedCalls)
        try {
            driver.handle("type", JSONObject().put("text", ""))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
            assertEquals("text required", e.message)
        }
    }

    @Test
    fun globalsValidateAndMintOnAcceptOnly() {
        val platform = FakePlatform(trees = listOf(read()))
        val driver = Driver(platform)
        driver.handle("back", JSONObject())
        driver.handle("home", JSONObject())
        assertEquals(listOf(GlobalAction.BACK, GlobalAction.HOME), platform.globals)
        platform.globalResult = false
        val before = driver.handle("snapshot", JSONObject()).getString("snapshot_id")
        try {
            driver.handle("back", JSONObject())
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertEquals(before, driver.handle("status", JSONObject()).getString("snapshot_id"))
        try {
            driver.handle("center", JSONObject().put("which", "control"))
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("which notification", e.message)
        }
    }

    @Test
    fun screenshotReturnsPngBase64AndKeepsLedger() {
        val platform = FakePlatform(trees = listOf(read()))
        val (driver, id) = mintedDriver(platform)
        val out = driver.handle("screenshot", JSONObject())
        val decoded = java.util.Base64.getDecoder().decode(out.getString("png_base64"))
        assertTrue(decoded.contentEquals(platform.png))
        assertEquals(id, driver.handle("status", JSONObject()).getString("snapshot_id"))
    }

    @Test
    fun staleRefFailsBeforeAnyTreeRead() {
        val platform = FakePlatform(trees = listOf(read()))
        val (driver, id) = mintedDriver(platform)
        val before = driver.handle("snapshot", JSONObject()).getString("snapshot_id")
        val afterSnapshotReads = platform.readCalls
        try {
            driver.handle("pinch", JSONObject().put("ref", "@$id:e1").put("scale", 2.0))
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
        assertEquals("stale ref must not trigger a fresh read", afterSnapshotReads, platform.readCalls)
        assertTrue(platform.gestures.isEmpty())
        assertEquals(before, driver.handle("status", JSONObject()).getString("snapshot_id"))
    }

    @Test
    fun explicitNullParamsRejected() {
        val driver = Driver(FakePlatform(trees = listOf(read())))
        val cases = listOf(
            "snapshot" to JSONObject().put("app", JSONObject.NULL),
            "hold" to JSONObject().put("x", 5).put("y", 5).put("duration", JSONObject.NULL),
            "pinch" to JSONObject().put("ref", "@x:e1").put("scale", 2.0).put("velocity", JSONObject.NULL),
            "tap" to JSONObject().put("x", JSONObject.NULL).put("y", 5),
            "type" to JSONObject().put("text", "hi").put("ref", JSONObject.NULL),
            "swipe" to JSONObject().put("direction", "up").put("ref", JSONObject.NULL),
        )
        for ((cmd, body) in cases) {
            try {
                driver.handle(cmd, body)
                fail("expected BAD_REQUEST for $cmd with null params")
            } catch (e: DriverException) {
                assertEquals("$cmd", "BAD_REQUEST", e.code)
            }
        }
    }

    @Test
    fun pointBoundaryRightEdgeRejected() {
        val platform = FakePlatform(trees = listOf(read()))
        val driver = Driver(platform)
        try {
            driver.handle("tap", JSONObject().put("x", 400).put("y", 20))
            fail("x == width must be outside")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
        try {
            driver.handle("tap", JSONObject().put("x", 20).put("y", 800))
            fail("y == height must be outside")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
        }
        driver.handle("tap", JSONObject().put("x", 0).put("y", 0))
        assertEquals(1, platform.gestures.size)
    }
}
