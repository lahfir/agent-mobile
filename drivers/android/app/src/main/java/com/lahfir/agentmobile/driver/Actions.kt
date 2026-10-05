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

    fun perform(
        target: RefTarget,
        action: NodeAction,
        cancellation: RequestCancellation,
    ): NodeActionResult = onTree(cancellation) { read, liveNodes ->
        val target = RefResolver.resolveOne(target, read, "ref", liveNodes) { it.model.identity }
        val accepted = when (action) {
            NodeAction.Click -> target.click()
            is NodeAction.AppendText -> target.appendText(action.text, cancellation)
            is NodeAction.Scroll -> target.scroll(action.direction)
        }
        NodeActionResult(read, target.model, accepted)
    }

    fun appendToFocused(text: String, cancellation: RequestCancellation): NodeActionResult = onTree(cancellation) { read, liveNodes ->
        if (!read.complete) {
            throw DriverException("DRIVER_ERROR", "live tree incomplete; cannot prove focused-node uniqueness")
        }
        val focused = liveNodes.filter {
            RefResolver.isActive(it.model.identity, read) &&
                "focused" in it.model.states && "Type" in it.model.availableActions
        }
        if (focused.isEmpty()) {
            throw DriverException("DRIVER_ERROR", "no focused editable node")
        }
        if (focused.size > 1) {
            throw DriverException("DRIVER_ERROR", "multiple focused editable nodes")
        }
        val target = focused[0]
        NodeActionResult(read, target.model, target.appendText(text, cancellation))
    }

    private fun <T> onTree(
        cancellation: RequestCancellation,
        block: (TreeRead, List<LiveNode>) -> T,
    ): T = service.onMain(cancellation) {
        val source = rootSource()
            ?: throw DriverException("DRIVER_ERROR", "no active accessibility window; focus a foreground app and retry")
        reader.useTree(source, 0L, densityProvider()) { read, liveNodes ->
            block(read.copy(readAtNanos = System.nanoTime()), liveNodes)
        }
    }
}
