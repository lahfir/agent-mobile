package com.lahfir.agentmobile.driver

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class RefLedgerTest {

    private var ids = 0
    private fun newId(): String = "snap%04d".format(++ids)

    private fun ledger(): RefLedger = RefLedger { newId() }

    private fun ident(
        className: String = "android.widget.Button",
        role: String = "button",
        resourceId: String = "",
        text: String = "",
        contentDescription: String = "",
        bounds: RawBounds = RawBounds(0, 0, 10, 10),
    ) = NodeIdentity(className, role, resourceId, text, contentDescription, bounds, packageName = "com.fake")

    private fun node(
        role: String = "button",
        name: String = "OK",
        value: String = "",
        states: List<String> = emptyList(),
        actions: List<String> = listOf("Tap"),
        bounds: LogicalBounds = LogicalBounds(0.0, 0.0, 5.0, 5.0),
        nativeId: NativeIdModel? = null,
        children: List<NodeModel> = emptyList(),
        identity: NodeIdentity = ident(),
    ) = NodeModel(role, name, value, states, actions, bounds, nativeId, children, identity)

    private fun read(
        root: NodeModel,
        complete: Boolean = true,
        signature: String = "sig",
        app: String = "com.fake",
    ) = TreeRead(app, root, complete, signature, RawBounds(0, 0, 0, 0), 1.0, 0L)

    @Test
    fun mintAssignsPreorderRefsAndSnapshotFields() {
        val ledger = ledger()
        val tree = node(children = listOf(node(name = "A"), node(name = "B", children = listOf(node(name = "C")))))
        val out = ledger.mint(read(tree), settled = true, reads = 2, settleMs = 150)

        assertEquals("com.fake", out.getString("app"))
        assertEquals("snap0001", out.getString("snapshot_id"))
        assertEquals(4, out.getInt("ref_count"))
        assertTrue(out.getBoolean("complete"))
        assertTrue(out.getBoolean("settled"))
        assertEquals(2, out.getInt("reads"))
        assertEquals(150, out.getInt("settle_ms"))
        assertEquals("snap0001", ledger.snapshotId)

        assertEquals("@snap0001:e1", out.getJSONObject("tree").getString("ref_id"))
        val kids = out.getJSONObject("tree").getJSONArray("children")
        assertEquals("@snap0001:e2", kids.getJSONObject(0).getString("ref_id"))
        assertEquals("@snap0001:e3", kids.getJSONObject(1).getString("ref_id"))
        assertEquals("@snap0001:e4", kids.getJSONObject(1).getJSONArray("children").getJSONObject(0).getString("ref_id"))

        val nodeJson = kids.getJSONObject(0)
        assertEquals("button", nodeJson.getString("role"))
        assertEquals("A", nodeJson.getString("name"))
        assertTrue(nodeJson.has("value"))
        assertTrue(nodeJson.has("states"))
        assertTrue(nodeJson.has("available_actions"))
        assertTrue(nodeJson.has("bounds"))
        assertTrue(nodeJson.has("children"))
        assertFalse(nodeJson.has("native_id"))
    }

    @Test
    fun nativeIdIncludedOnlyWhenPresent() {
        val ledger = ledger()
        val tree = node(children = listOf(node(nativeId = NativeIdModel("resource_id", "com.fake:id/x"))))
        val out = ledger.mint(read(tree), true, 1, 0)
        val child = out.getJSONObject("tree").getJSONArray("children").getJSONObject(0)
        assertEquals("resource_id", child.getJSONObject("native_id").getString("kind"))
        assertEquals("com.fake:id/x", child.getJSONObject("native_id").getString("value"))
    }

    @Test
    fun textRendersIndentedLinesWithoutTrailingNewline() {
        val ledger = ledger()
        val tree = node(
            name = "",
            actions = emptyList(),
            children = listOf(node(name = "A", states = listOf("focused"), value = "v")),
        )
        val out = ledger.mint(read(tree), true, 1, 0)
        val text = out.getString("text")
        assertEquals("@snap0001:e2 button \"A\" value=\"v\" at=0,0 size=5x5 [focused]", text)
    }

    @Test
    fun controlsInTextAreSanitized() {
        val ledger = ledger()
        val tree = node(name = "bad\nname\u0007", children = emptyList())
        val out = ledger.mint(read(tree), true, 1, 0)
        val text = out.getString("text")
        assertFalse(text.contains('\n'))
        assertTrue(text.contains('�'))
    }

    @Test
    fun newSnapshotSupersedesOldRefs() {
        val ledger = ledger()
        ledger.mint(read(node()), true, 1, 0)
        ledger.mint(read(node()), true, 1, 0)
        try {
            ledger.lookup("@snap0001:e1")
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
    }

    @Test
    fun lookupRejectsNonStringAndMalformedRefs() {
        val ledger = ledger()
        ledger.mint(read(node()), true, 1, 0)
        try {
            ledger.lookup(JSONObject.NULL)
            fail("expected BAD_REQUEST")
        } catch (e: DriverException) {
            assertEquals("BAD_REQUEST", e.code)
            assertEquals("ref required", e.message)
        }
        for (bad in listOf("nonsense", "@snap0001", "@snap0001:eX", "snap0001:e1", "@:e1")) {
            try {
                ledger.lookup(bad)
                fail("expected STALE_REF for $bad")
            } catch (e: DriverException) {
                assertEquals("STALE_REF", e.code)
            }
        }
    }

    @Test
    fun lookupRejectsUnknownSequenceAndOtherSnapshot() {
        val ledger = ledger()
        ledger.mint(read(node()), true, 1, 0)
        try {
            ledger.lookup("@snap0001:e9")
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
        try {
            ledger.lookup("@other999:e1")
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
    }

    @Test
    fun resolveMatchesExactlyAndWithinOnePixel() {
        val ledger = ledger()
        val id = ident(text = "save", bounds = RawBounds(100, 200, 150, 240))
        ledger.mint(read(node(identity = id)), true, 1, 0)
        val live = read(
            node(identity = ident(text = "save", bounds = RawBounds(101, 199, 151, 241))),
        )
        val resolved = ledger.resolve("@snap0001:e1", live)
        assertEquals("save", resolved.node.identity.text)
        assertEquals(live, resolved.baseline)
    }

    @Test
    fun resolveStaleOnZeroAndBeyondOnePixel() {
        val ledger = ledger()
        val id = ident(text = "save", bounds = RawBounds(100, 200, 150, 240))
        ledger.mint(read(node(identity = id)), true, 1, 0)
        try {
            ledger.resolve("@snap0001:e1", read(node(identity = ident(text = "save", bounds = RawBounds(102, 200, 150, 240)))))
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
        try {
            ledger.resolve("@snap0001:e1", read(node(identity = ident(text = "different"))))
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
    }

    @Test
    fun resolveAmbiguousOnDuplicateMatches() {
        val ledger = ledger()
        val id = ident(text = "dup")
        ledger.mint(read(node(identity = id)), true, 1, 0)
        val live = read(node(children = listOf(node(identity = id), node(identity = id))))
        try {
            ledger.resolve("@snap0001:e1", live)
            fail("expected AMBIGUOUS_TARGET")
        } catch (e: DriverException) {
            assertEquals("AMBIGUOUS_TARGET", e.code)
        }
    }

    @Test
    fun resolveRefusesIncompleteLiveTree() {
        val ledger = ledger()
        val id = ident(text = "x")
        ledger.mint(read(node(identity = id)), true, 1, 0)
        try {
            ledger.resolve("@snap0001:e1", read(node(identity = id), complete = false))
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
    }

    @Test
    fun invalidGeneratedIdThrowsAndPreservesLedger() {
        var calls = 0
        val ledger = RefLedger {
            calls += 1
            if (calls == 1) "valid001" else "NOT-VALID!"
        }
        ledger.mint(read(node(identity = ident(text = "keep"))), true, 1, 0)
        try {
            ledger.mint(read(node()), true, 1, 0)
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertEquals(17, calls)
        assertEquals("valid001", ledger.snapshotId)
        ledger.resolve("@valid001:e1", read(node(identity = ident(text = "keep"))))
    }

    @Test
    fun duplicateGeneratedIdRetriesThenErrors() {
        var calls = 0
        val ledger = RefLedger {
            calls += 1
            if (calls == 1) "first001" else "first001"
        }
        ledger.mint(read(node()), true, 1, 0)
        try {
            ledger.mint(read(node()), true, 1, 0)
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertEquals(17, calls)
        assertEquals("first001", ledger.snapshotId)
        ledger.resolve("@first001:e1", read(node(identity = ident())))
    }

    @Test
    fun duplicateThenFreshIdMintsSuccessfully() {
        var calls = 0
        val ledger = RefLedger {
            calls += 1
            when (calls) {
                1 -> "first001"
                2 -> "first001"
                else -> "second01"
            }
        }
        ledger.mint(read(node()), true, 1, 0)
        val out = ledger.mint(read(node()), true, 1, 0)
        assertEquals("second01", out.getString("snapshot_id"))
        assertEquals("second01", ledger.snapshotId)
    }

    @Test
    fun passwordTextNeverReachesJsonTextOrSignature() {
        val fakeRoot = LedgerFakeNode(children = listOf(
            LedgerFakeNode(
                className = "android.widget.EditText",
                isEditable = true,
                isPassword = true,
                text = "hunter2",
                contentDescription = "sentinel-desc",
                hintText = "sentinel-hint",
            ),
        ))
        val read1 = TreeReader().read(fakeRoot, 0L, 1.0)
        val pw = read1.root.children[0]
        assertEquals("", pw.name)
        assertEquals("", pw.value)
        assertEquals("", pw.identity.text)
        assertEquals("", pw.identity.contentDescription)
        val ledger = ledger()
        val out = ledger.mint(read1, true, 1, 0)
        val serialized = out.toString()
        for (sentinel in listOf("hunter2", "sentinel-desc", "sentinel-hint")) {
            assertFalse("$sentinel leaked into snapshot JSON", serialized.contains(sentinel))
            assertFalse("$sentinel leaked into text listing", out.getString("text").contains(sentinel))
        }

        val fakeRoot2 = LedgerFakeNode(children = listOf(
            LedgerFakeNode(
                className = "android.widget.EditText",
                isEditable = true,
                isPassword = true,
                text = "different-secret",
                contentDescription = "Password",
            ),
        ))
        val read2 = TreeReader().read(fakeRoot2, 0L, 1.0)
        assertEquals("password text must not affect the signature", read1.signature, read2.signature)
        assertTrue(read1.signature.matches(Regex("[0-9a-f]{64}")))
    }

    private class LedgerFakeNode(
        override var className: String = "android.widget.FrameLayout",
        override var packageName: String? = "com.fake",
        override var viewIdResourceName: String? = null,
        override var text: String? = null,
        override var contentDescription: String? = null,
        override var hintText: String? = null,
        override var isPassword: Boolean = false,
        override var isEditable: Boolean = false,
        override var isEnabled: Boolean = true,
        override var isSelected: Boolean = false,
        override var isFocused: Boolean = false,
        override var isCheckable: Boolean = false,
        override var isChecked: Boolean = false,
        override var isHeading: Boolean = false,
        override var isClickable: Boolean = false,
        override var isScrollable: Boolean = false,
        override var hasActionClick: Boolean = false,
        override var hasActionSetText: Boolean = false,
        override var hasActionScroll: Boolean = false,
        var bounds: RawBounds = RawBounds(0, 0, 100, 50),
        var children: List<LedgerFakeNode> = emptyList(),
    ) : NodeSource {
        override val boundsInScreen: RawBounds get() = bounds
        override val childCount get() = children.size
        override fun child(index: Int): NodeSource? = children.getOrNull(index)
        override fun sameNode(other: NodeSource): Boolean = this === other
        override val identityHash: Int get() = System.identityHashCode(this)
        override fun close() {}
    }

    @Test
    fun failedMintLeavesPreviousLedgerValid() {
        val ids2 = listOf("first001", "second01").iterator()
        val ledger = RefLedger { ids2.next() }
        ledger.mint(read(node(identity = ident(text = "keep"))), true, 1, 0)
        assertEquals("first001", ledger.snapshotId)
        val resolved = ledger.resolve("@first001:e1", read(node(identity = ident(text = "keep"))))
        assertEquals("keep", resolved.node.identity.text)
    }
}
