package com.lahfir.agentmobile.driver

import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.view.accessibility.AccessibilityNodeInfo
import java.security.MessageDigest

internal fun appendWithReturn(
    text: String,
    currentValue: () -> String,
    setText: (String) -> Boolean,
    imeEnter: (() -> Boolean)?,
    afterImeValue: (() -> String?)? = null,
    check: () -> Unit = {},
): Boolean {
    check()
    var current = currentValue()
    var index = 0
    while (index < text.length) {
        check()
        val newline = text.indexOf('\n', index)
        val span = if (newline < 0) text.substring(index) else text.substring(index, newline)
        if (span.isNotEmpty()) {
            check()
            if (!setText(current + span)) return false
            current += span
            check()
        }
        if (newline < 0) return true
        val nextIndex = newline + 1
        if (imeEnter != null) {
            check()
            if (!imeEnter()) return false
            check()
            if (nextIndex >= text.length) return true
            if (afterImeValue != null) {
                current = afterImeValue() ?: return false
                check()
            }
        } else {
            val next = current + "\n"
            check()
            if (!setText(next)) return false
            current = next
            check()
        }
        index = nextIndex
    }
    return true
}

internal interface NodeSource {
    val className: String
    val packageName: String?
    val viewIdResourceName: String?
    val text: String?
    val contentDescription: String?
    val hintText: String?
    val isPassword: Boolean
    val isEditable: Boolean
    val isEnabled: Boolean
    val isSelected: Boolean
    val isFocused: Boolean
    val isCheckable: Boolean
    val isChecked: Boolean
    val isHeading: Boolean
    val isClickable: Boolean
    val isScrollable: Boolean
    val hasActionClick: Boolean
    val hasActionSetText: Boolean
    val hasActionScroll: Boolean
    val boundsInScreen: RawBounds
    val childCount: Int
    fun child(index: Int): NodeSource?
    fun sameNode(other: NodeSource): Boolean
    val identityHash: Int
    val isVisibleToUser: Boolean get() = true
    val windowId: Int get() = 0
    fun performClick(): Boolean = false
    fun appendText(text: String, cancellation: RequestCancellation): Boolean = false
    fun performScroll(direction: String): Boolean = false
    fun close()
}

internal class LiveNode(val model: NodeModel, private val source: NodeSource) {
    fun click(): Boolean = source.performClick()
    fun appendText(text: String, cancellation: RequestCancellation): Boolean = source.appendText(text, cancellation)
    fun scroll(direction: String): Boolean = source.performScroll(direction)
}

private class AndroidNodeSource(private val node: AccessibilityNodeInfo) : NodeSource {
    override val className: String get() = node.className?.toString() ?: ""
    override val packageName: String? get() = node.packageName?.toString()
    override val viewIdResourceName: String? get() = node.viewIdResourceName
    override val text: String? get() = node.text?.toString()
    override val contentDescription: String? get() = node.contentDescription?.toString()
    override val hintText: String? get() = node.hintText?.toString()
    override val isPassword: Boolean get() = node.isPassword
    override val isEditable: Boolean get() = node.isEditable
    override val isEnabled: Boolean get() = node.isEnabled
    override val isSelected: Boolean get() = node.isSelected
    override val isFocused: Boolean get() = node.isFocused
    override val isCheckable: Boolean get() = node.isCheckable
    override val isChecked: Boolean get() = node.isChecked
    override val isHeading: Boolean get() = node.isHeading
    override val isClickable: Boolean get() = node.isClickable
    override val isScrollable: Boolean get() = node.isScrollable
    override val hasActionClick: Boolean
        get() = node.actionList.any { it.id == AccessibilityNodeInfo.ACTION_CLICK }
    override val hasActionSetText: Boolean
        get() = node.actionList.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT }
    override val hasActionScroll: Boolean
        get() = node.actionList.any { it.id in SCROLL_ACTION_IDS }
    override val boundsInScreen: RawBounds
        get() {
            val r = Rect()
            node.getBoundsInScreen(r)
            return RawBounds(r.left, r.top, r.right, r.bottom)
        }
    override val childCount: Int get() = node.childCount
    override fun child(index: Int): NodeSource? = node.getChild(index)?.let { AndroidNodeSource(it) }
    override fun sameNode(other: NodeSource): Boolean =
        (other as? AndroidNodeSource)?.let { node == it.node } ?: false
    override val identityHash: Int get() = node.hashCode()
    override val isVisibleToUser: Boolean get() = node.isVisibleToUser
    override val windowId: Int get() = node.windowId

    override fun performClick(): Boolean =
        (node.isClickable || node.actionList.any { it.id == AccessibilityNodeInfo.ACTION_CLICK }) &&
            node.performAction(AccessibilityNodeInfo.ACTION_CLICK)

    override fun appendText(text: String, cancellation: RequestCancellation): Boolean {
        if (!node.isEditable || node.actionList.none { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT }) {
            return false
        }
        val imeEnter = if (
            node.actionList.any {
                it.id == AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id
            }
        ) {
            ({ node.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id) })
        } else {
            null
        }
        return appendWithReturn(
            text,
            currentValue = { node.text?.toString() ?: "" },
            setText = { value ->
                val args = Bundle()
                args.putCharSequence(
                    AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                    value,
                )
                node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
            },
            imeEnter = imeEnter,
            afterImeValue = {
                if (node.refresh()) node.text?.toString() ?: "" else null
            },
            check = cancellation::check,
        )
    }

    override fun performScroll(direction: String): Boolean {
        val candidates = when (direction) {
            "up" -> listOf(
                AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP,
                AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_UP,
                AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_FORWARD,
            )
            "down" -> listOf(
                AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN,
                AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_DOWN,
                AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_BACKWARD,
            )
            "left" -> listOf(
                AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_LEFT,
                AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_LEFT,
            )
            "right" -> listOf(
                AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_RIGHT,
                AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_RIGHT,
            )
            else -> return false
        }
        for (action in candidates) {
            if (node.actionList.any { it.id == action.id } && node.performAction(action.id)) {
                return true
            }
        }
        return false
    }

    @Suppress("DEPRECATION")
    override fun close() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            node.recycle()
        }
    }

    companion object {
        private val SCROLL_ACTION_IDS = setOf(
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_FORWARD.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_BACKWARD.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_LEFT.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_RIGHT.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_TO_POSITION.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_UP.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_DOWN.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_LEFT.id,
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_RIGHT.id,
        )
    }
}

internal class TreeReader(
    private val maxNodes: Int = 4096,
    private val maxDepth: Int = 128,
) {

    fun read(rootSource: NodeSource, readAtNanos: Long, density: Double): TreeRead =
        useTree(rootSource, readAtNanos, density) { read, _ -> read }

    fun <T> useTree(
        rootSource: NodeSource,
        readAtNanos: Long,
        density: Double,
        block: (TreeRead, List<LiveNode>) -> T,
    ): T {
        val obtained = mutableListOf(rootSource)
        val (read, liveNodes) = try {
            readTree(rootSource, readAtNanos, density, obtained)
        } catch (e: DriverException) {
            closeAll(obtained)
            throw e
        } catch (e: Exception) {
            closeAll(obtained)
            throw DriverException("DRIVER_ERROR", "tree read failed: ${e.javaClass.simpleName}")
        }
        try {
            return block(read, liveNodes)
        } finally {
            closeAll(obtained)
        }
    }

    private fun closeAll(obtained: List<NodeSource>) {
        obtained.forEach { source ->
            try {
                source.close()
            } catch (_: Exception) {
            }
        }
    }

    private fun readTree(
        rootSource: NodeSource,
        readAtNanos: Long,
        density: Double,
        obtained: MutableList<NodeSource>,
    ): Pair<TreeRead, List<LiveNode>> {
        var complete = true
        val visited = mutableMapOf<Int, MutableList<NodeSource>>()
        val entries = mutableListOf<Triple<NodeSource, Int, Int>>()
        val stack = ArrayDeque<Triple<NodeSource, Int, Int>>()
        stack.addLast(Triple(rootSource, 0, -1))
        var remainingNodeReads = (maxNodes - 1).coerceAtLeast(0)

        while (stack.isNotEmpty()) {
            val (source, depth, parent) = stack.removeLast()
            if (entries.size >= maxNodes) {
                complete = false
                break
            }
            val bucket = visited.getOrPut(source.identityHash) { mutableListOf() }
            if (bucket.any { it.sameNode(source) }) {
                complete = false
                continue
            }
            bucket += source
            val index = entries.size
            entries += Triple(source, depth, parent)
            val count = source.childCount
            if (depth >= maxDepth) {
                if (count > 0) {
                    complete = false
                }
                continue
            }
            val take = minOf(count, remainingNodeReads)
            if (count > take) complete = false
            for (i in take - 1 downTo 0) {
                remainingNodeReads -= 1
                val child = source.child(i)
                if (child == null) {
                    complete = false
                    continue
                }
                obtained += child
                stack.addLast(Triple(child, depth + 1, index))
            }
        }
        if (stack.isNotEmpty()) {
            complete = false
        }
        if (entries.isEmpty()) {
            throw DriverException("DRIVER_ERROR", "tree read produced no nodes")
        }

        val models = arrayOfNulls<NodeModel>(entries.size)
        val childrenLists = Array(entries.size) { ArrayDeque<NodeModel>() }
        val originBounds = entries[0].first.boundsInScreen
        val effectiveDensity = if (density.isFinite() && density > 0) density else 1.0
        for (i in entries.indices.reversed()) {
            val (source, _, parent) = entries[i]
            val model = modelNode(source, childrenLists[i].toList(), originBounds, effectiveDensity)
            models[i] = model
            if (parent >= 0) {
                childrenLists[parent].addFirst(model)
            }
        }
        val root = models[0]!!
        val read = TreeRead(
            app = entries[0].first.packageName?.toString() ?: "",
            root = root,
            complete = complete,
            signature = signature(root),
            origin = originBounds,
            density = effectiveDensity,
            readAtNanos = readAtNanos,
        )
        val liveNodes = entries.mapIndexed { i, entry -> LiveNode(models[i]!!, entry.first) }
        return read to liveNodes
    }

    private fun modelNode(
        source: NodeSource,
        children: List<NodeModel>,
        origin: RawBounds,
        density: Double,
    ): NodeModel {
        val className = source.className
        val password = source.isPassword
        val editable = source.isEditable || className.endsWith("EditText")
        val role = roleFor(className, editable, password)
        val contentDescription = if (password) "" else (source.contentDescription?.toString() ?: "")
        val text = if (password) "" else (source.text?.toString() ?: "")
        val hint = if (password) "" else (source.hintText?.toString() ?: "")
        val name = when {
            password -> ""
            contentDescription.isNotEmpty() -> contentDescription
            !editable -> text
            hint.isNotEmpty() -> hint
            else -> ""
        }
        val value = when {
            password -> ""
            editable -> text
            contentDescription.isNotEmpty() && contentDescription != text -> text
            else -> ""
        }
        val states = mutableListOf<String>()
        if (!source.isEnabled) states += "disabled"
        if (source.isSelected) states += "selected"
        if (source.isFocused) states += "focused"
        if (source.isCheckable) states += if (source.isChecked) "checked" else "unchecked"
        if (source.isHeading) states += "heading"
        val actions = mutableListOf<String>()
        if (source.hasActionClick || source.isClickable) actions += "Tap"
        if (editable && source.hasActionSetText) actions += "Type"
        if (source.isScrollable || source.hasActionScroll) actions += "Swipe"
        val resourceId = source.viewIdResourceName ?: ""
        val raw = source.boundsInScreen
        val logical = LogicalBounds(
            x = (raw.left - origin.left) / density,
            y = (raw.top - origin.top) / density,
            width = ((raw.right - raw.left) / density).coerceAtLeast(0.0),
            height = ((raw.bottom - raw.top) / density).coerceAtLeast(0.0),
        )
        return NodeModel(
            role = role,
            name = name,
            value = value,
            states = states,
            availableActions = actions,
            bounds = logical,
            nativeId = if (resourceId.isNotBlank()) NativeIdModel("resource_id", resourceId) else null,
            children = children,
            identity = NodeIdentity(
                className = className,
                role = role,
                resourceId = resourceId,
                text = text,
                contentDescription = contentDescription,
                rawBounds = raw,
                packageName = source.packageName?.toString() ?: "",
                visibleToUser = source.isVisibleToUser,
                windowId = source.windowId,
            ),
        )
    }

    private fun roleFor(className: String, editable: Boolean, password: Boolean): String = when {
        className.endsWith("EditText") && editable -> if (password) "securetextfield" else "textfield"
        className.endsWith("RadioButton") -> "radio"
        className.endsWith("ToggleButton") -> "toggle"
        className.endsWith("Switch") -> "switch"
        className.endsWith("CheckBox") -> "checkbox"
        className.endsWith("Button") -> "button"
        className.endsWith("TextView") -> "text"
        className.endsWith("ImageView") -> "image"
        className.endsWith("SeekBar") -> "slider"
        className.endsWith("ProgressBar") -> "progress"
        className.endsWith("Spinner") -> "picker"
        className.endsWith("DatePicker") -> "datepicker"
        className.endsWith("RecyclerView") || className.endsWith("GridView") -> "collectionview"
        className.endsWith("ListView") -> "table"
        className.endsWith("ScrollView") -> "scrollview"
        className.endsWith("WebView") -> "webview"
        className.endsWith("Toolbar") -> "toolbar"
        className.endsWith("SearchView") -> "searchfield"
        else -> "group"
    }

    private fun signature(root: NodeModel): String {
        val digest = MessageDigest.getInstance("SHA-256")
        fun putInt(value: Int) {
            digest.update(
                byteArrayOf(
                    (value shr 24).toByte(),
                    (value shr 16).toByte(),
                    (value shr 8).toByte(),
                    value.toByte(),
                ),
            )
        }
        fun putString(value: String) {
            val bytes = value.toByteArray(Charsets.UTF_8)
            putInt(bytes.size)
            digest.update(bytes)
        }
        fun walk(node: NodeModel) {
            putString(node.identity.className)
            putString(node.role)
            putString(node.identity.packageName)
            digest.update(if (node.identity.visibleToUser) 1.toByte() else 0.toByte())
            putInt(node.identity.windowId)
            putInt(node.identity.rawBounds.left)
            putInt(node.identity.rawBounds.top)
            putInt(node.identity.rawBounds.right)
            putInt(node.identity.rawBounds.bottom)
            putString(node.name)
            putString(node.nativeId?.kind ?: "")
            putString(node.nativeId?.value ?: "")
            putString(node.value)
            putInt(node.children.size)
            node.children.forEach(::walk)
        }
        walk(root)
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    companion object {
        internal fun createPlatformSource(node: AccessibilityNodeInfo): NodeSource = AndroidNodeSource(node)

        internal fun readPackageName(node: AccessibilityNodeInfo): String {
            val source = AndroidNodeSource(node)
            try {
                return source.packageName?.toString() ?: ""
            } finally {
                source.close()
            }
        }
    }
}
