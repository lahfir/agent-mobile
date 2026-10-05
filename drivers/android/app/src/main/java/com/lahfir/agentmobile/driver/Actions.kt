package com.lahfir.agentmobile.driver

internal sealed interface NodeAction {
    data object Click : NodeAction
    data class AppendText(val text: String) : NodeAction
    data class Scroll(val direction: String) : NodeAction
}

internal enum class GlobalAction {
    BACK,
    HOME,
    NOTIFICATIONS,
}

internal data class NodeActionResult(
    val read: TreeRead,
    val node: NodeModel,
    val accepted: Boolean,
)

internal class Actions(
    private val service: AgentMobileAccessibilityService,
    private val reader: TreeReader,
    private val rootSource: () -> NodeSource? = {
        service.rootInActiveWindow?.let(TreeReader::createPlatformSource)
    },
    private val densityProvider: () -> Double = {
        service.resources.displayMetrics.density.toDouble()
    },
) {

    fun perform(identity: NodeIdentity, action: NodeAction): NodeActionResult = onTree { read, liveNodes ->
        if (!read.complete) {
            throw DriverException("DRIVER_ERROR", "live tree incomplete; cannot prove ref uniqueness")
        }
        val hits = liveNodes.filter { RefResolver.matches(identity, it.model.identity) }
        if (hits.isEmpty()) {
            throw DriverException("STALE_REF", "ref no longer matches a live element; re-snapshot")
        }
        if (hits.size > 1) {
            throw DriverException("AMBIGUOUS_TARGET", "ref matches ${hits.size} live elements; re-snapshot")
        }
        val target = hits[0]
        val accepted = when (action) {
            NodeAction.Click -> target.click()
            is NodeAction.AppendText -> target.appendText(action.text)
            is NodeAction.Scroll -> target.scroll(action.direction)
        }
        NodeActionResult(read, target.model, accepted)
    }

    fun appendToFocused(text: String): NodeActionResult = onTree { read, liveNodes ->
        if (!read.complete) {
            throw DriverException("DRIVER_ERROR", "live tree incomplete; cannot prove focused-node uniqueness")
        }
        val focused = liveNodes.filter {
            "focused" in it.model.states && "Type" in it.model.availableActions
        }
        if (focused.isEmpty()) {
            throw DriverException("DRIVER_ERROR", "no focused editable node")
        }
        if (focused.size > 1) {
            throw DriverException("DRIVER_ERROR", "multiple focused editable nodes")
        }
        val target = focused[0]
        NodeActionResult(read, target.model, target.appendText(text))
    }

    private fun <T> onTree(block: (TreeRead, List<LiveNode>) -> T): T = service.onMain {
        val source = rootSource()
            ?: throw DriverException("DRIVER_ERROR", "no active accessibility window; focus a foreground app and retry")
        reader.useTree(source, 0L, densityProvider()) { read, liveNodes ->
            block(read.copy(readAtNanos = System.nanoTime()), liveNodes)
        }
    }
}
