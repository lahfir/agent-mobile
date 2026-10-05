package com.lahfir.agentmobile.driver

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class TreeReaderTest {

    private val created = mutableListOf<FakeNode>()

    private class FakeNode(
        val id: Int = nextId++,
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
        var children: List<FakeNode> = emptyList(),
        var throwOnAccess: Boolean = false,
    ) : NodeSource {
        var closed = false

        private fun gate() {
            if (throwOnAccess) throw IllegalStateException("cannot access a recycled node")
        }

        override val boundsInScreen: RawBounds get() {
            gate()
            return bounds
        }
        override val childCount get() = children.size
        var childCalls = 0
        override fun child(index: Int): NodeSource? {
            gate()
            childCalls += 1
            return children.getOrNull(index)
        }
        override fun sameNode(other: NodeSource): Boolean =
            (other as? FakeNode)?.id == id
        override val identityHash: Int get() = id
        override fun close() {
            closed = true
        }

        companion object {
            private var nextId = 1
        }
    }

    private fun fake(block: FakeNode.() -> Unit = {}): FakeNode {
        val n = FakeNode()
        n.block()
        created += n
        return n
    }

    private fun read(root: NodeSource, density: Double = 1.0, maxNodes: Int = 4096, maxDepth: Int = 128): TreeRead =
        TreeReader(maxNodes, maxDepth).read(root, readAtNanos = 1_000L, density = density)

    @Test
    fun logicalBoundsSubtractRootOriginAndDivideByDensity() {
        val root = fake {
            bounds = RawBounds(10, 20, 110, 120)
            packageName = "com.fake"
            children = listOf(
                fake { bounds = RawBounds(30, 60, 50, 80) },
            )
        }
        val read = read(root, density = 2.0)
        assertEquals(0.0, read.root.bounds.x, 0.0001)
        assertEquals(0.0, read.root.bounds.y, 0.0001)
        assertEquals(50.0, read.root.bounds.width, 0.0001)
        assertEquals(50.0, read.root.bounds.height, 0.0001)
        val child = read.root.children[0]
        assertEquals(10.0, child.bounds.x, 0.0001)
        assertEquals(20.0, child.bounds.y, 0.0001)
        assertEquals(10.0, child.bounds.width, 0.0001)
        assertEquals(10.0, child.bounds.height, 0.0001)
        assertEquals(RawBounds(30, 60, 50, 80), child.identity.rawBounds)
        assertEquals(RawBounds(10, 20, 110, 120), read.origin)
        assertEquals("com.fake", read.app)
    }

    @Test
    fun resourceIdEmitsNativeIdAndAbsentIdOmitsIt() {
        val root = fake {
            children = listOf(
                fake { viewIdResourceName = "com.fake:id/save" },
                fake { viewIdResourceName = "" },
            )
        }
        val read = read(root)
        assertEquals(NativeIdModel("resource_id", "com.fake:id/save"), read.root.children[0].nativeId)
        assertNull(read.root.children[1].nativeId)
    }

    @Test
    fun passwordNodeRedactsValueAndIdentityText() {
        val root = fake {
            children = listOf(
                fake {
                    className = "android.widget.EditText"
                    isEditable = true
                    isPassword = true
                    text = "hunter2"
                    contentDescription = "Password"
                },
            )
        }
        val read = read(root)
        val pw = read.root.children[0]
        assertEquals("securetextfield", pw.role)
        assertEquals("", pw.value)
        assertEquals("", pw.identity.text)
        assertEquals("", pw.identity.contentDescription)
    }

    private class SecretSource : NodeSource {
        var textReads = 0
        var descReads = 0
        var hintReads = 0
        override val className = "android.widget.EditText"
        override val packageName = "com.fake"
        override val viewIdResourceName = null
        override val text: String? get() { textReads += 1; return "s3cret-text" }
        override val contentDescription: String? get() { descReads += 1; return "s3cret-desc" }
        override val hintText: String? get() { hintReads += 1; return "s3cret-hint" }
        override val isPassword = true
        override val isEditable = true
        override val isEnabled = true
        override val isSelected = false
        override val isFocused = false
        override val isCheckable = false
        override val isChecked = false
        override val isHeading = false
        override val isClickable = false
        override val isScrollable = false
        override val hasActionClick = false
        override val hasActionSetText = false
        override val hasActionScroll = false
        override val boundsInScreen = RawBounds(0, 0, 10, 10)
        override val childCount = 0
        override fun child(index: Int) = null
        override fun sameNode(other: NodeSource) = other === this
        override val identityHash = 42
        override fun close() {}
    }

    @Test
    fun passwordNodeNeverEvaluatesSecretGetters() {
        val src = SecretSource()
        val read = TreeReader().read(
            object : NodeSource {
                override val className = "android.widget.FrameLayout"
                override val packageName = "com.fake"
                override val viewIdResourceName = null
                override val text = null
                override val contentDescription = null
                override val hintText = null
                override val isPassword = false
                override val isEditable = false
                override val isEnabled = true
                override val isSelected = false
                override val isFocused = false
                override val isCheckable = false
                override val isChecked = false
                override val isHeading = false
                override val isClickable = false
                override val isScrollable = false
                override val hasActionClick = false
                override val hasActionSetText = false
                override val hasActionScroll = false
                override val boundsInScreen = RawBounds(0, 0, 10, 10)
                override val childCount = 1
                override fun child(index: Int): NodeSource = src
                override fun sameNode(other: NodeSource) = other === this
                override val identityHash = 7
                override fun close() {}
            },
            0L,
            1.0,
        )
        assertEquals("password text must never be read", 0, src.textReads)
        assertEquals("password contentDescription must never be read", 0, src.descReads)
        assertEquals("password hint must never be read", 0, src.hintReads)
        val pw = read.root.children[0]
        assertEquals("", pw.name)
        assertEquals("", pw.value)
        assertEquals("", pw.identity.text)
        assertEquals("", pw.identity.contentDescription)
    }

    @Test
    fun roleMappingCoversCoreClasses() {
        fun roleOf(cls: String, password: Boolean = false, editable: Boolean = true): String {
            val node = fake {
                className = cls
                isEditable = editable
                isPassword = password
            }
            val read = read(fake { children = listOf(node) })
            return read.root.children[0].role
        }
        assertEquals("textfield", roleOf("android.widget.EditText"))
        assertEquals("textfield", roleOf("androidx.appcompat.widget.AppCompatEditText"))
        assertEquals("securetextfield", roleOf("android.widget.EditText", password = true))
        assertEquals("button", roleOf("android.widget.Button"))
        assertEquals("button", roleOf("android.widget.ImageButton"))
        assertEquals("text", roleOf("android.widget.TextView"))
        assertEquals("image", roleOf("android.widget.ImageView"))
        assertEquals("checkbox", roleOf("android.widget.CheckBox"))
        assertEquals("radio", roleOf("android.widget.RadioButton"))
        assertEquals("switch", roleOf("android.widget.Switch"))
        assertEquals("toggle", roleOf("android.widget.ToggleButton"))
        assertEquals("slider", roleOf("android.widget.SeekBar"))
        assertEquals("progress", roleOf("android.widget.ProgressBar"))
        assertEquals("picker", roleOf("android.widget.Spinner"))
        assertEquals("datepicker", roleOf("android.widget.DatePicker"))
        assertEquals("collectionview", roleOf("androidx.recyclerview.widget.RecyclerView"))
        assertEquals("collectionview", roleOf("android.widget.GridView"))
        assertEquals("table", roleOf("android.widget.ListView"))
        assertEquals("scrollview", roleOf("android.widget.ScrollView"))
        assertEquals("scrollview", roleOf("androidx.core.widget.NestedScrollView"))
        assertEquals("webview", roleOf("android.webkit.WebView"))
        assertEquals("toolbar", roleOf("androidx.appcompat.widget.Toolbar"))
        assertEquals("searchfield", roleOf("android.widget.SearchView"))
        assertEquals("group", roleOf("com.unknown.Widget"))
    }

    @Test
    fun nameValueAndStatesFollowRules() {
        val root = fake {
            children = listOf(
                fake { className = "android.widget.TextView"; text = "shown" },
                fake { className = "android.widget.TextView"; text = "body"; contentDescription = "label" },
                fake { className = "android.widget.EditText"; isEditable = true; text = "typed"; hintText = "hint here" },
                fake { className = "android.widget.CheckBox"; isCheckable = true; isChecked = true; isEnabled = false; isSelected = true; isFocused = true; isHeading = true },
            )
        }
        val read = read(root)
        val c = read.root.children
        assertEquals("shown", c[0].name)
        assertEquals("", c[0].value)
        assertEquals("label", c[1].name)
        assertEquals("body", c[1].value)
        assertEquals("hint here", c[2].name)
        assertEquals("typed", c[2].value)
        assertEquals(listOf("disabled", "selected", "focused", "checked", "heading"), c[3].states)
    }

    @Test
    fun actionsFollowAdvertisedCapabilities() {
        val root = fake {
            children = listOf(
                fake { isClickable = true },
                fake { className = "android.widget.EditText"; isEditable = true; hasActionSetText = true },
                fake { isScrollable = true },
                fake { hasActionClick = true; hasActionScroll = true },
            )
        }
        val read = read(root)
        val c = read.root.children
        assertEquals(listOf("Tap"), c[0].availableActions)
        assertEquals(listOf("Type"), c[1].availableActions)
        assertEquals(listOf("Swipe"), c[2].availableActions)
        assertEquals(listOf("Tap", "Swipe"), c[3].availableActions)
    }

    @Test
    fun childOrderIsPreservedAndSignatureIsDeterministic() {
        fun build() = fake {
            children = listOf(
                fake { text = "first" },
                fake { text = "second" },
            )
        }
        val a = read(build())
        val b = read(build())
        assertEquals("first", a.root.children[0].name)
        assertEquals("second", a.root.children[1].name)
        assertEquals(a.signature, b.signature)
        assertNotEquals(a.signature, read(build().also { it.children[1].text = "changed" }).signature)
    }

    @Test
    fun repeatedNodeIdentityMarksIncompleteWithoutRecursing() {
        val shared = fake { text = "shared" }
        val root = fake { children = listOf(shared, shared) }
        val read = read(root)
        assertFalse(read.complete)
        assertEquals(1, read.root.children.size)
    }

    @Test
    fun nodeCapMarksIncomplete() {
        val root = fake {
            children = listOf(fake(), fake(), fake())
        }
        val read = read(root, maxNodes = 3)
        assertFalse(read.complete)
    }

    @Test
    fun nodeCapBoundsChildMaterialization() {
        val fanOut = 500
        val root = fake {
            children = (1..fanOut).map { i -> fake { text = "child-$i" } }
        }
        val budget = 8
        val read = read(root, maxNodes = budget)
        assertFalse(read.complete)
        assertEquals(budget - 1, root.childCalls)
        assertEquals(budget, countNodes(read.root))
        assertTrue(root.closed)
        assertTrue(root.children.take(root.childCalls).all { it.closed })
        assertTrue(root.children.drop(root.childCalls).all { !it.closed })
        assertEquals((1 until budget).map { "child-$it" }, read.root.children.map { it.name })
    }

    private fun countNodes(node: com.lahfir.agentmobile.driver.NodeModel): Int =
        1 + node.children.sumOf(::countNodes)

    @Test
    fun depthCapMarksIncomplete() {
        val leaf = fake()
        val mid = fake { children = listOf(leaf) }
        val root = fake { children = listOf(mid) }
        val read = read(root, maxDepth = 1)
        assertFalse(read.complete)
    }

    @Test
    fun leafAtExactlyMaxDepthIsComplete() {
        val leaf = fake()
        val root = fake { children = listOf(leaf) }
        val read = read(root, maxDepth = 1)
        assertTrue(read.complete)
        assertEquals(1, read.root.children.size)
    }

    @Test
    fun valueOmitsTextWhenContentDescriptionMatches() {
        val root = fake {
            children = listOf(
                fake {
                    className = "android.widget.TextView"
                    text = "Save"
                    contentDescription = "Save"
                },
                fake {
                    className = "android.widget.TextView"
                    text = "body"
                    contentDescription = "label"
                },
            )
        }
        val read = read(root)
        val same = read.root.children[0]
        assertEquals("Save", same.name)
        assertEquals("", same.value)
        val distinct = read.root.children[1]
        assertEquals("label", distinct.name)
        assertEquals("body", distinct.value)
    }

    @Test
    fun sourceAccessFailureMapsDriverErrorAndClosesObtainedSources() {
        val bad = fake { throwOnAccess = true }
        val root = fake { children = listOf(bad) }
        try {
            read(root)
            fail("expected DriverException")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertTrue(created.all { it.closed })
    }

    @Test
    fun propertyAccessFailureMapsDriverError() {
        val root = fake { throwOnAccess = true }
        try {
            read(root)
            fail("expected DriverException")
        } catch (e: DriverException) {
            assertEquals("DRIVER_ERROR", e.code)
        }
        assertTrue(created.all { it.closed })
    }

    @Test
    fun allObtainedSourcesAreClosedAfterSuccessfulRead() {
        val root = fake { children = listOf(fake()) }
        read(root)
        assertTrue(created.all { it.closed })
    }
}

class AppendWithReturnTest {

    @Test
    fun newlineUsesImeEnterWhenAvailable() {
        val sets = mutableListOf<String>()
        var ime = 0
        var value = "cur"
        val ok = appendWithReturn(
            "hello\n",
            currentValue = { value },
            setText = { sets += it; value = it; true },
            imeEnter = { ime += 1; true },
        )
        assertTrue(ok)
        assertEquals(listOf("curhello"), sets)
        assertEquals(1, ime)
    }

    @Test
    fun imeNewlineUpdatesModeledValueThroughRefresh() {
        val calls = mutableListOf<String>()
        var value = ""
        val ok = appendWithReturn(
            "a\nb",
            currentValue = { value },
            setText = { calls += "set:$it"; value = it; true },
            imeEnter = { calls += "ime"; value += "\n"; true },
            afterImeValue = { value },
        )
        assertTrue(ok)
        assertEquals(listOf("set:a", "ime", "set:a\nb"), calls)
    }

    @Test
    fun imeWithoutModeledNewlineNeverSynthesizesOne() {
        val calls = mutableListOf<String>()
        var value = ""
        val ok = appendWithReturn(
            "a\nb",
            currentValue = { value },
            setText = { calls += "set:$it"; value = it; true },
            imeEnter = { calls += "ime"; true },
            afterImeValue = { value },
        )
        assertTrue(ok)
        assertEquals(listOf("set:a", "ime", "set:ab"), calls)
    }

    @Test
    fun imeRefreshFailureStopsAppend() {
        val calls = mutableListOf<String>()
        var value = ""
        val ok = appendWithReturn(
            "a\nb",
            currentValue = { value },
            setText = { calls += "set:$it"; value = it; true },
            imeEnter = { calls += "ime"; true },
            afterImeValue = { null },
        )
        assertFalse(ok)
        assertEquals(listOf("set:a", "ime"), calls)
    }

    @Test
    fun cancellationAfterFirstSetStopsBeforeIme() {
        val cancellation = RequestCancellation()
        val sets = mutableListOf<String>()
        var value = ""
        var imeCalls = 0
        val ok = try {
            appendWithReturn(
                "a\nb",
                currentValue = { value },
                setText = {
                    sets += it
                    value = it
                    cancellation.cancel()
                    true
                },
                imeEnter = { imeCalls += 1; true },
                check = cancellation::check,
            )
            "completed"
        } catch (e: DriverException) {
            e.code
        }
        assertEquals("DRIVER_ERROR", ok)
        assertEquals(listOf("a"), sets)
        assertEquals(0, imeCalls)
    }

    @Test
    fun successfulImeNewlineRemainsInNextSubmittedValue() {
        val calls = mutableListOf<String>()
        var value = ""
        val ok = appendWithReturn(
            "a\nb",
            currentValue = { value },
            setText = { calls += it; value = it; true },
            imeEnter = { value += "\n"; true },
            afterImeValue = { value },
        )
        assertTrue(ok)
        assertEquals(listOf("a", "a\nb"), calls)
    }

    @Test
    fun trailingNewlineSkipsImeValueRefresh() {
        var refreshes = 0
        var value = "cur"
        val ok = appendWithReturn(
            "a\n",
            currentValue = { value },
            setText = { value = it; true },
            imeEnter = { true },
            afterImeValue = { refreshes += 1; null },
        )
        assertTrue(ok)
        assertEquals(0, refreshes)
    }

    @Test
    fun newlineFallsBackToLiteralWhenNoImeEnter() {
        val sets = mutableListOf<String>()
        var value = "x"
        val ok = appendWithReturn(
            "a\nb",
            currentValue = { value },
            setText = { sets += it; value = it; true },
            imeEnter = null,
        )
        assertTrue(ok)
        assertEquals(listOf("xa", "xa\n", "xa\nb"), sets)
    }

    @Test
    fun refusedStepStopsRemainingCalls() {
        val sets = mutableListOf<String>()
        var value = ""
        var ime = 0
        var calls = 0
        val ok = appendWithReturn(
            "a\nb\nc",
            currentValue = { value },
            setText = {
                calls += 1
                sets += it
                value = it
                calls < 2
            },
            imeEnter = { ime += 1; true },
        )
        assertFalse(ok)
        assertEquals(1, ime)
        assertEquals(2, calls)
    }
}

class IdentityHashTest {

    private class CountingSource(
        private val hash: Int,
        private val onCompare: () -> Unit,
        var same: (Any?) -> Boolean = { false },
        private val kids: List<NodeSource> = emptyList(),
        private val pkg: String = "com.fake",
    ) : NodeSource {
        override val className = "android.widget.FrameLayout"
        override var packageName: String? = pkg
        override var viewIdResourceName: String? = null
        override var text: String? = null
        override var contentDescription: String? = null
        override var hintText: String? = null
        override var isPassword = false
        override var isEditable = false
        override var isEnabled = true
        override var isSelected = false
        override var isFocused = false
        override var isCheckable = false
        override var isChecked = false
        override var isHeading = false
        override var isClickable = false
        override var isScrollable = false
        override var hasActionClick = false
        override var hasActionSetText = false
        override var hasActionScroll = false
        override val boundsInScreen = RawBounds(0, 0, 100, 50)
        override val childCount get() = kids.size
        override fun child(index: Int): NodeSource? = kids.getOrNull(index)
        override fun sameNode(other: NodeSource): Boolean {
            onCompare()
            return same(other)
        }
        override val identityHash: Int get() = hash
        override fun close() {}
    }

    @Test
    fun nearCapUniqueHashesKeepComparisonsLinear() {
        var compares = 0
        fun node(hash: Int, kids: List<NodeSource> = emptyList()): CountingSource =
            CountingSource(hash, { compares += 1 }, { false }, kids)
        // 64-deep chain of unique hashes: worst case is one bucket lookup per node.
        var root = node(1)
        for (i in 2..64) {
            root = node(i, listOf(root))
        }
        val read = TreeReader().read(root, 0L, 1.0)
        assertTrue(read.complete)
        assertTrue(
            "64 unique-hash nodes must stay linear, saw $compares",
            compares <= 64,
        )
    }

    @Test
    fun sameHashCollisionStillDistinguishesViaSameNode() {
        var compares = 0
        val first = CountingSource(42, { compares += 1 })
        val second = CountingSource(42, { compares += 1 })
        first.same = { it === first }
        second.same = { it === second }
        val dupOfFirst = CountingSource(42, { compares += 1 })
        first.same = { it === first || it === dupOfFirst }
        val root = CountingSource(7, { compares += 1 }, kids = listOf(first, second, dupOfFirst))
        val read = TreeReader().read(root, 0L, 1.0)
        assertEquals("sameNode must distinguish same-hash nodes", 2, read.root.children.size)
    }

    @Test
    fun staleRereadCannotCorruptSubsequentSpans() {
        var reads = 0
        val sets = mutableListOf<String>()
        val ok = appendWithReturn(
            "a\nb",
            currentValue = { reads += 1; "base" },
            setText = { sets += it; true },
            imeEnter = { true },
            afterImeValue = { "basea\n" },
        )
        assertTrue(ok)
        assertEquals("currentValue must be read exactly once", 1, reads)
        assertEquals(listOf("basea", "basea\nb"), sets)
    }
}
