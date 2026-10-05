package com.lahfir.agentmobile.driver

internal data class RawBounds(val left: Int, val top: Int, val right: Int, val bottom: Int) {
    val width: Int get() = right - left
    val height: Int get() = bottom - top
}

internal data class LogicalBounds(val x: Double, val y: Double, val width: Double, val height: Double)

internal data class NativeIdModel(val kind: String, val value: String)

internal data class NodeIdentity(
    val className: String,
    val role: String,
    val resourceId: String,
    val text: String,
    val contentDescription: String,
    val rawBounds: RawBounds,
)

internal data class NodeModel(
    val role: String,
    val name: String,
    val value: String,
    val states: List<String>,
    val availableActions: List<String>,
    val bounds: LogicalBounds,
    val nativeId: NativeIdModel?,
    val children: List<NodeModel>,
    val identity: NodeIdentity,
)

internal data class TreeRead(
    val app: String,
    val root: NodeModel,
    val complete: Boolean,
    val signature: String,
    val origin: RawBounds,
    val density: Double,
    val readAtNanos: Long,
)
