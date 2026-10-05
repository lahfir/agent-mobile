package com.lahfir.agentmobile.driver

import android.accessibilityservice.AccessibilityService
import android.os.Build
import org.json.JSONObject
import kotlin.math.abs

internal interface DriverPlatform {
    fun status(): PlatformStatus
    fun readTree(): TreeRead
    fun performNodeAction(identity: NodeIdentity, action: NodeAction): NodeActionResult
    fun appendToFocused(text: String): NodeActionResult
    fun dispatchGesture(spec: GestureSpec)
    fun performGlobal(action: GlobalAction): Boolean
    fun screenshot(): ByteArray
}

internal data class PlatformStatus(val app: String, val device: String, val os: String)

private class AndroidDriverPlatform(private val service: AgentMobileAccessibilityService) : DriverPlatform {
    private val reader = TreeReader()
    private val actions = Actions(service, reader)
    private val gestures = GestureDispatcher(AndroidGestureSubmission(service))
    private val screenshots = Screenshots(AndroidScreenshotBackend(service))

    override fun status(): PlatformStatus = service.onMain {
        PlatformStatus(
            app = service.rootInActiveWindow?.let { TreeReader.readPackageName(it) } ?: "",
            device = Build.MODEL ?: "",
            os = Build.VERSION.RELEASE ?: "",
        )
    }

    override fun readTree(): TreeRead = service.onMain {
        val node = service.rootInActiveWindow
            ?: throw DriverException("DRIVER_ERROR", "no active accessibility window; focus a foreground app and retry")
        val density = service.resources.displayMetrics.density.toDouble()
        reader.read(
            TreeReader.createPlatformSource(node),
            readAtNanos = 0L,
            density = density,
        ).copy(readAtNanos = System.nanoTime())
    }

    override fun performNodeAction(identity: NodeIdentity, action: NodeAction): NodeActionResult =
        actions.perform(identity, action)

    override fun appendToFocused(text: String): NodeActionResult = actions.appendToFocused(text)

    override fun dispatchGesture(spec: GestureSpec) {
        gestures.dispatch(spec)
    }

    override fun performGlobal(action: GlobalAction): Boolean = service.onMain {
        service.performGlobalAction(
            when (action) {
                GlobalAction.BACK -> AccessibilityService.GLOBAL_ACTION_BACK
                GlobalAction.HOME -> AccessibilityService.GLOBAL_ACTION_HOME
                GlobalAction.NOTIFICATIONS -> AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS
            },
        )
    }

    override fun screenshot(): ByteArray = screenshots.capture()
}

internal class Driver internal constructor(
    private val platform: DriverPlatform,
    private val settler: Settler = Settler(),
) {

    constructor(service: AgentMobileAccessibilityService) : this(
        AndroidDriverPlatform(service),
        Settler(),
    )

    internal val ledger = RefLedger()
    private var lastRead: TreeRead? = null

    fun handle(command: String, params: JSONObject): JSONObject = when (command) {
        "status" -> status()
        "snapshot" -> snapshot(params)
        "tap" -> tap(params)
        "doubletap" -> doubletap(params)
        "hold" -> hold(params)
        "pinch" -> pinch(params)
        "twofinger" -> twofinger(params)
        "type" -> type(params)
        "swipe" -> swipe(params)
        "back" -> global(GlobalAction.BACK)
        "home" -> global(GlobalAction.HOME)
        "center" -> center(params)
        "screenshot" -> screenshot()
        else -> throw DriverException("UNKNOWN_COMMAND", "unknown command: $command")
    }

    private fun status(): JSONObject {
        val current = platform.status()
        return JSONObject()
            .put("app", current.app)
            .put("snapshot_id", ledger.snapshotId)
            .put("device", current.device)
            .put("os", current.os)
    }

    private fun snapshot(params: JSONObject): JSONObject {
        val requestedApp = when {
            !params.has("app") -> null
            else -> params.opt("app").let {
                if (it is String && it.isNotEmpty()) {
                    it
                } else {
                    throw DriverException("BAD_REQUEST", "app must be a nonempty string")
                }
            }
        }
        val result = settler.settle(lastRead) { platform.readTree() }
        if (requestedApp != null && requestedApp != result.read.app) {
            throw DriverException("BAD_REQUEST", "app $requestedApp is not the foreground package ${result.read.app}")
        }
        val out = ledger.mint(result.read, result.settled, result.reads, result.settleMs)
        lastRead = result.read
        return out
    }

    private fun mutate(baseline: TreeRead?): JSONObject {
        val result = settler.settle(baseline ?: lastRead) { platform.readTree() }
        val out = ledger.mint(result.read, result.settled, result.reads, result.settleMs)
        lastRead = result.read
        return out
    }

    private fun pointOrNull(params: JSONObject): RawPoint? {
        val hasX = params.has("x")
        val hasY = params.has("y")
        if (!hasX && !hasY) {
            return null
        }
        if (!hasX || !hasY) {
            throw DriverException("BAD_REQUEST", "x and y must be sent together")
        }
        val x = params.opt("x")
        val y = params.opt("y")
        if (x !is Number || y !is Number) {
            throw DriverException("BAD_REQUEST", "x and y must be sent together")
        }
        if (!x.toDouble().isFinite() || !y.toDouble().isFinite()) {
            throw DriverException("BAD_REQUEST", "x and y must be finite")
        }
        return RawPoint(x.toFloat(), y.toFloat())
    }

    private fun pointInRoot(point: RawPoint, read: TreeRead): RawPoint {
        val root = read.root.bounds
        if (point.x < 0 || point.y < 0 || point.x >= root.width || point.y >= root.height) {
            throw DriverException("BAD_REQUEST", "x and y must be inside the active window")
        }
        return Gestures.toRaw(point, read)
    }

    private fun resolveRef(any: Any?): ResolvedNode {
        val identity = ledger.lookup(any)
        val live = platform.readTree()
        return ResolvedNode(RefResolver.resolve(identity, live, any.toString()), live)
    }

    private fun gestureTarget(params: JSONObject): Pair<RawPoint, TreeRead> {
        val point = pointOrNull(params)
        if (point != null) {
            val live = platform.readTree()
            return pointInRoot(point, live) to live
        }
        val resolved = resolveRef(params.opt("ref"))
        return Gestures.rawCenter(resolved.node.identity.rawBounds) to resolved.baseline
    }

    private fun tap(params: JSONObject): JSONObject {
        val point = pointOrNull(params)
        if (point != null) {
            val live = platform.readTree()
            platform.dispatchGesture(Gestures.tap(pointInRoot(point, live)))
            return mutate(live)
        }
        val identity = ledger.lookup(params.opt("ref"))
        val result = platform.performNodeAction(identity, NodeAction.Click)
        if (!result.accepted) {
            platform.dispatchGesture(Gestures.tap(Gestures.rawCenter(result.node.identity.rawBounds)))
        }
        return mutate(result.read)
    }

    private fun doubletap(params: JSONObject): JSONObject {
        val (center, baseline) = gestureTarget(params)
        platform.dispatchGesture(Gestures.doubleTap(center))
        return mutate(baseline)
    }

    private fun hold(params: JSONObject): JSONObject {
        val seconds = when {
            !params.has("duration") -> 1.0
            else -> params.opt("duration").let {
                if (it is Number) it.toDouble() else throw DriverException("BAD_REQUEST", "duration must be a number")
            }
        }
        if (!seconds.isFinite() || seconds <= 0 || seconds > 10) {
            throw DriverException("BAD_REQUEST", "duration 0 < d <= 10")
        }
        val (center, baseline) = gestureTarget(params)
        platform.dispatchGesture(Gestures.hold(center, (seconds * 1000).toLong()))
        return mutate(baseline)
    }

    private fun pinch(params: JSONObject): JSONObject {
        val rawScale = params.opt("scale")
        val scale = rawScale as? Number
        if (scale == null || !scale.toDouble().isFinite() ||
            scale.toDouble() <= 0 || abs(scale.toDouble() - 1.0) < 0.01
        ) {
            throw DriverException("BAD_REQUEST", "scale positive, finite, |scale-1| >= 0.01")
        }
        val velocity = when {
            !params.has("velocity") -> 0.0
            else -> params.opt("velocity").let {
                if (it is Number && it.toDouble().isFinite()) {
                    it.toDouble()
                } else {
                    throw DriverException("BAD_REQUEST", "velocity finite")
                }
            }
        }
        val resolved = resolveRef(params.opt("ref"))
        platform.dispatchGesture(Gestures.pinch(resolved.node.identity.rawBounds, scale.toDouble(), velocity))
        return mutate(resolved.baseline)
    }

    private fun twofinger(params: JSONObject): JSONObject {
        val resolved = resolveRef(params.opt("ref"))
        platform.dispatchGesture(Gestures.twoFinger(resolved.node.identity.rawBounds))
        return mutate(resolved.baseline)
    }

    private fun type(params: JSONObject): JSONObject {
        val text = params.opt("text")
        if (text !is String || text.isEmpty()) {
            throw DriverException("BAD_REQUEST", "text required")
        }
        val result = if (params.has("ref")) {
            platform.performNodeAction(ledger.lookup(params.opt("ref")), NodeAction.AppendText(text))
        } else {
            platform.appendToFocused(text)
        }
        if (!result.accepted) {
            throw DriverException("DRIVER_ERROR", "type action not accepted by target")
        }
        return mutate(result.read)
    }

    private fun swipe(params: JSONObject): JSONObject {
        val direction = params.opt("direction")
        if (direction !is String || direction !in setOf("up", "down", "left", "right")) {
            throw DriverException("BAD_REQUEST", "direction up|down|left|right")
        }
        if (params.has("ref")) {
            val result = platform.performNodeAction(ledger.lookup(params.opt("ref")), NodeAction.Scroll(direction))
            if (!result.accepted) {
                platform.dispatchGesture(Gestures.swipe(result.node.identity.rawBounds, direction, 0.6))
            }
            return mutate(result.read)
        }
        val live = platform.readTree()
        platform.dispatchGesture(Gestures.swipe(live.origin, direction, 0.5))
        return mutate(live)
    }

    private fun global(action: GlobalAction): JSONObject {
        if (!platform.performGlobal(action)) {
            throw DriverException("DRIVER_ERROR", "${action.name.lowercase()} action refused")
        }
        return mutate(null)
    }

    private fun center(params: JSONObject): JSONObject {
        val which = params.opt("which")
        if (which !is String || which != "notification") {
            throw DriverException("BAD_REQUEST", "which notification")
        }
        return global(GlobalAction.NOTIFICATIONS)
    }

    private fun screenshot(): JSONObject = JSONObject().put(
        "png_base64",
        java.util.Base64.getEncoder().encodeToString(platform.screenshot()),
    )
}
