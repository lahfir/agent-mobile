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
        override fun child(index: Int): NodeSource? {
            gate()
            return children.getOrNull(index)
        }
        override fun sameNode(other: NodeSource): Boolean =
            (other as? FakeNode)?.id == id
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
