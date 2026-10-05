package com.lahfir.agentmobile.driver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class ActionsTest {

    private class FakeSource(
        val id: Int,
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
        var children: List<FakeSource> = emptyList(),
        var clickResult: Boolean = true,
        var appendResult: Boolean = true,
        var scrollResult: Boolean = true,
    ) : NodeSource {
        var closed = false
        var clicks = 0
        var appends = 0
        var appendedText: String? = null
        var scrolls = 0
        var scrolledDirection: String? = null

        override val boundsInScreen: RawBounds get() = bounds
        override val childCount get() = children.size
        override fun child(index: Int): NodeSource? = children.getOrNull(index)
        override fun sameNode(other: NodeSource): Boolean = (other as? FakeSource)?.id == id
        override fun performClick(): Boolean { clicks += 1; return clickResult }
        override fun appendText(text: String): Boolean { appends += 1; appendedText = text; return appendResult }
        override fun performScroll(direction: String): Boolean { scrolls += 1; scrolledDirection = direction; return scrollResult }
        override fun close() { closed = true }
    }

    private fun actions(root: NodeSource?, density: Double = 1.0): Actions =
        Actions(
            AgentMobileAccessibilityService(),
            TreeReader(),
            rootSource = { root },
            densityProvider = { density },
        )

    @Test
    fun useTreePairsModelsWithSourcesPreorderAndClosesAll() {
        val leaf = FakeSource(3, text = "leaf")
        val mid = FakeSource(2, children = listOf(leaf))
        val root = FakeSource(1, children = listOf(mid))
        TreeReader().useTree(root, 0L, 1.0) { read, liveNodes ->
            assertEquals(3, liveNodes.size)
            assertSame(read.root, liveNodes[0].model)
            assertSame(read.root.children[0], liveNodes[1].model)
            assertSame(read.root.children[0].children[0], liveNodes[2].model)
            liveNodes
        }
        assertTrue(root.closed && mid.closed && leaf.closed)
    }

    @Test
    fun useTreeClosesSourcesWhenBlockThrows() {
        val child = FakeSource(2)
        val root = FakeSource(1, children = listOf(child))
        try {
            TreeReader().useTree(root, 0L, 1.0) { _, _ -> throw IllegalStateException("boom") }
            fail("expected throw")
        } catch (e: IllegalStateException) {
            assertEquals("boom", e.message)
        }
        assertTrue(root.closed && child.closed)
    }

    @Test
    fun semanticClickActsOnUniqueResolvedSource() {
        val target = FakeSource(2, className = "android.widget.Button", isClickable = true, hasActionClick = true)
        val root = FakeSource(1, children = listOf(target))
        val result = actions(root).perform(
            NodeIdentity("android.widget.Button", "button", "", "", "", target.bounds),
            NodeAction.Click,
        )
        assertTrue(result.accepted)
        assertEquals(1, target.clicks)
        assertTrue(root.closed && target.closed)
    }

    @Test
    fun semanticClickZeroMatchThrowsStaleRef() {
        val root = FakeSource(1)
        try {
            actions(root).perform(
                NodeIdentity("android.widget.Button", "button", "", "", "", RawBounds(9, 9, 9, 9)),
                NodeAction.Click,
            )
            fail("expected STALE_REF")
        } catch (e: DriverException) {
            assertEquals("STALE_REF", e.code)
        }
    }

    @Test
    fun ambiguousIdentityFailsBeforeAction() {
        val a = FakeSource(2, className = "android.widget.TextView", text = "same")
        val b = FakeSource(3, className = "android.widget.TextView", text = "same")
        val root = FakeSource(1, children = listOf(a, b))
        try {
            actions(root).perform(
                NodeIdentity("android.widget.TextView", "text", "", "same", "", a.bounds),
                NodeAction.Click,
            )
            fail("expected AMBIGUOUS_TARGET")
        } catch (e: DriverException) {
            assertEquals("AMBIGUOUS_TARGET", e.code)
        }
        assertEquals(0, a.clicks + b.clicks)
    }

    @Test
    fun incompleteTreeRefusesBeforeAction() {
        val shared = FakeSource(2)
        val root = FakeSource(1, children = listOf(shared, shared))
        try {
            actions(root).perform(NodeIdentity("", "group", "", "", "", RawBounds(0, 0, 0, 0)), NodeAction.Click)
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
    }

    @Test
    fun unacceptedClickReportsNotAccepted() {
        val target = FakeSource(2, className = "android.widget.Button", clickResult = false)
        val root = FakeSource(1, children = listOf(target))
        val result = actions(root).perform(
            NodeIdentity("android.widget.Button", "button", "", "", "", target.bounds),
            NodeAction.Click,
        )
        assertFalse(result.accepted)
        assertEquals(target.bounds, result.node.identity.rawBounds)
    }

    @Test
    fun appendToFocusedRequiresExactlyOneFocusedEditable() {
        val focused = FakeSource(
            2,
            className = "android.widget.EditText",
            isEditable = true,
            isFocused = true,
            hasActionSetText = true,
        )
        val root = FakeSource(1, children = listOf(focused))
        val result = actions(root).appendToFocused("hello")
        assertTrue(result.accepted)
        assertEquals("hello", focused.appendedText)
    }

    @Test
    fun appendToFocusedFailsWithZeroOrMultiple() {
        val editable = { id: Int ->
            FakeSource(id, className = "android.widget.EditText", isEditable = true, isFocused = true, hasActionSetText = true)
        }
        try {
            actions(FakeSource(1)).appendToFocused("x")
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        val a = editable(2)
        val b = editable(3)
        val root = FakeSource(1, children = listOf(a, b))
        try {
            actions(root).appendToFocused("x")
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertEquals(0, a.appends + b.appends)
    }

    @Test
    fun appendToFocusedRejectsIncompleteTree() {
        val shared = FakeSource(
            7,
            className = "android.widget.EditText",
            isEditable = true,
            isFocused = true,
            hasActionSetText = true,
        )
        val root = FakeSource(1, children = listOf(shared, shared))
        try {
            actions(root).appendToFocused("x")
            fail("expected DRIVER_ERROR")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertEquals("must not act on an incomplete tree", 0, shared.appends)
    }

    @Test
    fun scrollDelegatesDirectionToSource() {
        val target = FakeSource(2, isScrollable = true, hasActionScroll = true, bounds = RawBounds(10, 10, 90, 90))
        val root = FakeSource(1, children = listOf(target))
        val result = actions(root).perform(
            NodeIdentity("android.widget.FrameLayout", "group", "", "", "", target.bounds),
            NodeAction.Scroll("down"),
        )
        assertTrue(result.accepted)
        assertEquals("down", target.scrolledDirection)
    }
}
