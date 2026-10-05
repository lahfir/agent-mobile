package com.lahfir.agentmobile.driver

import java.security.SecureRandom
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs

internal data class ResolvedNode(val node: NodeModel, val baseline: TreeRead)

internal data class RefTarget(val app: String, val identity: NodeIdentity)

internal object RefResolver {

    fun resolve(target: RefTarget, live: TreeRead, ref: String): NodeModel {
        val nodes = mutableListOf<NodeModel>()
        fun walk(node: NodeModel) {
            nodes += node
            node.children.forEach(::walk)
        }
        walk(live.root)
        return resolveOne(target, live, ref, nodes) { it.identity }
    }

    fun <T> resolveOne(
        target: RefTarget,
        live: TreeRead,
        ref: String,
        candidates: List<T>,
        identity: (T) -> NodeIdentity,
    ): T {
        if (target.app != live.app) {
            throw DriverException("STALE_REF", "ref $ref belongs to ${target.app}, not ${live.app}; re-snapshot")
        }
        if (!live.complete) {
            throw DriverException("DRIVER_ERROR", "live tree incomplete; cannot prove ref uniqueness")
        }
        val matches = candidates.filter { matches(target.identity, identity(it)) }
        if (matches.isEmpty()) {
            throw DriverException("STALE_REF", "ref $ref no longer matches a live element; re-snapshot")
        }
        if (matches.size > 1) {
            throw DriverException("AMBIGUOUS_TARGET", "ref $ref matches ${matches.size} live elements; re-snapshot")
        }
        requireActive(identity(matches[0]), live, ref)
        return matches[0]
    }

    fun isActive(identity: NodeIdentity, live: TreeRead): Boolean {
        val root = live.root.identity
        return identity.visibleToUser && identity.packageName == live.app && identity.windowId == root.windowId
    }

    private fun requireActive(identity: NodeIdentity, live: TreeRead, ref: String) {
        if (!isActive(identity, live)) {
            throw DriverException("STALE_REF", "ref $ref is not visible in the active app window; re-snapshot")
        }
    }

    fun matches(a: NodeIdentity, b: NodeIdentity): Boolean =
        a.className == b.className &&
            a.role == b.role &&
            a.resourceId == b.resourceId &&
            a.text == b.text &&
            a.contentDescription == b.contentDescription &&
            a.packageName == b.packageName &&
            a.visibleToUser == b.visibleToUser &&
            a.windowId == b.windowId &&
            abs(a.rawBounds.left - b.rawBounds.left) <= 1 &&
            abs(a.rawBounds.top - b.rawBounds.top) <= 1 &&
            abs(a.rawBounds.right - b.rawBounds.right) <= 1 &&
            abs(a.rawBounds.bottom - b.rawBounds.bottom) <= 1
}

internal class RefLedger(
    private val newSnapshotId: () -> String = { generateSnapshotId() },
) {
    private var refs: Map<String, RefTarget> = emptyMap()

    private var snapshotApp: String? = null

    var snapshotId: String = ""
        private set

    fun mint(read: TreeRead, settled: Boolean, reads: Int, settleMs: Long): JSONObject {
        var chosen: String? = null
        var attempts = 0
        while (chosen == null && attempts < MAX_ID_ATTEMPTS) {
            attempts += 1
            val candidate = newSnapshotId()
            if (ID_PATTERN.matches(candidate) && candidate != snapshotId) {
                chosen = candidate
            }
        }
        val id = chosen
            ?: throw DriverException("DRIVER_ERROR", "snapshot id generator failed to produce a fresh valid id")
        val newRefs = LinkedHashMap<String, RefTarget>()
        val lines = mutableListOf<String>()
        var sequence = 0

        fun serialize(node: NodeModel, printedDepth: Int): JSONObject {
            sequence += 1
            val ref = "@$id:e$sequence"
            newRefs[ref] = RefTarget(read.app, node.identity)
            val printed = node.name.isNotEmpty() || node.value.isNotEmpty() || node.availableActions.isNotEmpty()
            if (printed) {
                var line = "  ".repeat(printedDepth) + ref + " " + node.role + " \"" + clean(node.name) + "\""
                if (node.value.isNotEmpty()) {
                    line += " value=\"" + clean(node.value) + "\""
                }
                line += " at=" + node.bounds.x.toLong() + "," + node.bounds.y.toLong() +
                    " size=" + node.bounds.width.toLong() + "x" + node.bounds.height.toLong()
                if (node.states.isNotEmpty()) {
                    line += " [" + node.states.joinToString(",") + "]"
                }
                lines += line
            }
            val json = JSONObject()
                .put("role", node.role)
                .put("name", node.name)
                .put("value", node.value)
                .put("ref_id", ref)
                .put("states", JSONArray(node.states))
                .put("available_actions", JSONArray(node.availableActions))
                .put(
                    "bounds",
                    JSONObject()
                        .put("x", node.bounds.x)
                        .put("y", node.bounds.y)
                        .put("width", node.bounds.width)
                        .put("height", node.bounds.height),
                )
            node.nativeId?.let {
                json.put("native_id", JSONObject().put("kind", it.kind).put("value", it.value))
            }
            val children = JSONArray()
            node.children.forEach { child ->
                children.put(serialize(child, if (printed) printedDepth + 1 else printedDepth))
            }
            json.put("children", children)
            return json
        }

        val treeJson = serialize(read.root, 0)
        val out = JSONObject()
            .put("app", read.app)
            .put("snapshot_id", id)
            .put("ref_count", sequence)
            .put("complete", read.complete)
            .put("settled", settled)
            .put("reads", reads)
            .put("settle_ms", settleMs)
            .put("text", lines.joinToString("\n"))
            .put("tree", treeJson)
        refs = newRefs
        snapshotApp = read.app
        snapshotId = id
        return out
    }

    fun lookup(any: Any?): RefTarget {
        if (any !is String) {
            throw DriverException("BAD_REQUEST", "ref required")
        }
        val (id, _) = parseRef(any)
            ?: throw DriverException("STALE_REF", "ref $any is not a valid snapshot ref; re-snapshot")
        if (id != snapshotId) {
            throw DriverException("STALE_REF", "current snapshot is @$snapshotId; ref $any is from another snapshot; re-snapshot")
        }
        return refs[any]
            ?: throw DriverException("STALE_REF", "ref $any no longer matches a live element; re-snapshot")
    }

    fun invalidateIfAppChanged(app: String) {
        if (snapshotApp != null && snapshotApp != app) {
            refs = emptyMap()
            snapshotApp = null
            snapshotId = ""
        }
    }

    fun resolve(any: Any?, live: TreeRead): ResolvedNode {
        val target = lookup(any)
        return ResolvedNode(RefResolver.resolve(target, live, any.toString()), live)
    }

    private fun parseRef(ref: String): Pair<String, Int>? {
        val rest = ref.removePrefix("@")
        if (rest == ref) return null
        val sep = rest.indexOf(':')
        if (sep <= 0) return null
        val id = rest.substring(0, sep)
        val seq = rest.substring(sep + 1)
        if (id.isEmpty() || !seq.startsWith("e")) return null
        val index = seq.substring(1).toIntOrNull() ?: return null
        if (index <= 0) return null
        return id to index
    }

    private fun clean(value: String): String = buildString {
        for (c in value) {
            append(if (c.isISOControl()) '�' else c)
        }
    }

    companion object {
        private val ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789".toCharArray()
        private val ID_PATTERN = Regex("[a-z0-9]{8}")
        private const val MAX_ID_ATTEMPTS = 16
        private val RANDOM = SecureRandom()

        fun generateSnapshotId(): String = buildString {
            repeat(8) { append(ALPHABET[RANDOM.nextInt(ALPHABET.size)]) }
        }
    }
}
