package com.lahfir.agentmobile.driver

import android.accessibilityservice.AccessibilityService
import android.os.Build
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.abs

internal interface DriverPlatform {
    fun status(): PlatformStatus
    fun readTree(): TreeRead
    fun performNodeAction(target: RefTarget, action: NodeAction, cancellation: RequestCancellation): NodeActionResult
    fun appendToFocused(text: String, cancellation: RequestCancellation): NodeActionResult
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

    override fun performNodeAction(
        target: RefTarget,
        action: NodeAction,
        cancellation: RequestCancellation,
    ): NodeActionResult = actions.perform(target, action, cancellation)

    override fun appendToFocused(text: String, cancellation: RequestCancellation): NodeActionResult =
        actions.appendToFocused(text, cancellation)

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
    private val admission = AtomicBoolean(false)

    fun handle(
        command: String,
        params: JSONObject,
        cancellation: RequestCancellation = RequestCancellation(),
    ): JSONObject {
        cancellation.check()
        if (!admission.compareAndSet(false, true)) {
            throw DriverBusyException()
        }
        try {
            val out = when (command) {
                "status" -> status(cancellation)
                "snapshot" -> snapshot(params, cancellation)
                "tap" -> tap(params, cancellation)
                "doubletap" -> doubletap(params, cancellation)
                "hold" -> hold(params, cancellation)
                "pinch" -> pinch(params, cancellation)
                "twofinger" -> twofinger(params, cancellation)
                "type" -> type(params, cancellation)
                "swipe" -> swipe(params, cancellation)
                "back" -> global(GlobalAction.BACK, cancellation)
                "home" -> global(GlobalAction.HOME, cancellation)
                "center" -> center(params, cancellation)
                "screenshot" -> screenshot(cancellation)
                else -> throw DriverException("UNKNOWN_COMMAND", "unknown command: $command")
            }
            cancellation.check()
            return out
        } finally {
            admission.set(false)
        }
    }

    /** Check cancellation before and after every platform operation. */
    private fun <T> checked(cancellation: RequestCancellation, block: () -> T): T {
        cancellation.check()
        val result = block()
        cancellation.check()
        return result
    }

    private fun status(cancellation: RequestCancellation): JSONObject {
        val current = checked(cancellation) { platform.status() }
        if (current.app.isBlank()) {
            throw DriverException("DRIVER_ERROR", "no active accessibility window; focus a foreground app and retry")
        }
        ledger.invalidateIfAppChanged(current.app)
        return JSONObject()
            .put("app", current.app)
            .put("snapshot_id", ledger.snapshotId)
            .put("device", current.device)
            .put("os", current.os)
    }

    private fun snapshot(params: JSONObject, cancellation: RequestCancellation): JSONObject {
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
        val result = settler.settle(lastRead) { checked(cancellation) { platform.readTree() } }
        if (requestedApp != null && requestedApp != result.read.app) {
            throw DriverException("BAD_REQUEST", "app $requestedApp is not the foreground package ${result.read.app}")
        }
        val out = ledger.mint(result.read, result.settled, result.reads, result.settleMs)
        lastRead = result.read
        return out
    }

    private fun mutate(baseline: TreeRead?, cancellation: RequestCancellation): JSONObject {
        val result = settler.settle(baseline ?: lastRead) { checked(cancellation) { platform.readTree() } }
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



    private fun resolveRef(any: Any?, cancellation: RequestCancellation): ResolvedNode {
        return ledger.resolve(any) { checked(cancellation) { platform.readTree() } }
    }

    /** A logical point converted to raw must still land inside the active
        window — density-correct containment, never clipped. */
    private fun checkedPoint(point: RawPoint, live: TreeRead): RawPoint {
        val raw = Gestures.toRaw(point, live)
        if (!live.origin.contains(raw)) {
            throw DriverException("BAD_REQUEST", "x and y must be inside the active window")
        }
        return raw
    }

    private fun checkedRootGesture(spec: GestureSpec, live: TreeRead): GestureSpec {
        if (spec.strokes.flatMap { it.points }.any { !live.origin.contains(it) }) {
            throw DriverException("STALE_REF", "gesture leaves the active app window; re-snapshot")
        }
        return spec
    }

    private fun checkedRefGesture(resolved: ResolvedNode, spec: GestureSpec): GestureSpec {
        val id = resolved.node.identity
        val rootId = resolved.baseline.root.identity
        if (!id.visibleToUser || id.packageName != resolved.baseline.app || id.windowId != rootId.windowId) {
            throw DriverException("STALE_REF", "ref target is not visible in the active app window; re-snapshot")
        }
        if (spec.strokes.flatMap { it.points }.any { !resolved.baseline.origin.contains(it) }) {
            throw DriverException("STALE_REF", "ref target gesture leaves the active app window; re-snapshot")
        }
        return spec
    }

    private fun tap(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val point = pointOrNull(params)
        if (point != null) {
            val live = checked(cancellation) { platform.readTree() }
            checked(cancellation) { platform.dispatchGesture(Gestures.tap(checkedPoint(point, live))) }
            return mutate(live, cancellation)
        }
        val result = checked(cancellation) {
            platform.performNodeAction(ledger.lookup(params.opt("ref")), NodeAction.Click, cancellation)
        }
        if (!result.accepted) {
            val resolved = ResolvedNode(result.node, result.read)
            checked(cancellation) {
                platform.dispatchGesture(
                    checkedRefGesture(resolved, Gestures.tap(Gestures.rawCenter(result.node.identity.rawBounds))),
                )
            }
        }
        return mutate(result.read, cancellation)
    }

    private fun doubletap(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val point = pointOrNull(params)
        if (point != null) {
            val live = checked(cancellation) { platform.readTree() }
            checked(cancellation) { platform.dispatchGesture(Gestures.doubleTap(checkedPoint(point, live))) }
            return mutate(live, cancellation)
        }
        val resolved = resolveRef(params.opt("ref"), cancellation)
        val spec = checkedRefGesture(
            resolved,
            Gestures.doubleTap(Gestures.rawCenter(resolved.node.identity.rawBounds)),
        )
        checked(cancellation) { platform.dispatchGesture(spec) }
        return mutate(resolved.baseline, cancellation)
    }

    private fun hold(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val seconds = when {
            !params.has("duration") -> 1.0
            else -> params.opt("duration").let {
                if (it is Number) it.toDouble() else throw DriverException("BAD_REQUEST", "duration must be a number")
            }
        }
        if (!seconds.isFinite() || seconds <= 0 || seconds > 10) {
            throw DriverException("BAD_REQUEST", "duration 0 < d <= 10")
        }
        val point = pointOrNull(params)
        if (point != null) {
            val live = checked(cancellation) { platform.readTree() }
            checked(cancellation) {
                platform.dispatchGesture(Gestures.hold(checkedPoint(point, live), (seconds * 1000).toLong()))
            }
            return mutate(live, cancellation)
        }
        val resolved = resolveRef(params.opt("ref"), cancellation)
        val spec = checkedRefGesture(
            resolved,
            Gestures.hold(Gestures.rawCenter(resolved.node.identity.rawBounds), (seconds * 1000).toLong()),
        )
        checked(cancellation) { platform.dispatchGesture(spec) }
        return mutate(resolved.baseline, cancellation)
    }

    private fun pinch(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val rawScale = params.opt("scale")
        val scale = rawScale as? Number
        if (scale == null || !scale.toDouble().isFinite() ||
            scale.toDouble() <= 0 || abs(scale.toDouble() - 1.0) < 0.01
        ) {
            throw DriverException("BAD_REQUEST", "scale positive, finite, |scale-1| >= 0.01")
        }
        val velocity = pinchVelocity(params, scale.toDouble())
        val resolved = resolveRef(params.opt("ref"), cancellation)
        checked(cancellation) {
            platform.dispatchGesture(
                checkedRefGesture(resolved, Gestures.pinch(resolved.node.identity.rawBounds, scale.toDouble(), velocity)),
            )
        }
        return mutate(resolved.baseline, cancellation)
    }

    /** Omitted velocity defaults to +1 for spread and -1 for converge —
        the iOS contract; an explicit finite zero stays explicit zero. */
    internal fun pinchVelocity(params: JSONObject, scale: Double): Double = when {
        !params.has("velocity") -> if (scale > 1.0) 1.0 else -1.0
        else -> params.opt("velocity").let {
            if (it is Number && it.toDouble().isFinite()) {
                it.toDouble()
            } else {
                throw DriverException("BAD_REQUEST", "velocity finite")
            }
        }
    }

    private fun twofinger(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val resolved = resolveRef(params.opt("ref"), cancellation)
        checked(cancellation) {
            platform.dispatchGesture(
                checkedRefGesture(resolved, Gestures.twoFinger(resolved.node.identity.rawBounds)),
            )
        }
        return mutate(resolved.baseline, cancellation)
    }

    private fun type(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val text = params.opt("text")
        if (text !is String || text.isEmpty()) {
            throw DriverException("BAD_REQUEST", "text required")
        }
        val result = if (params.has("ref")) {
            checked(cancellation) {
                platform.performNodeAction(ledger.lookup(params.opt("ref")), NodeAction.AppendText(text), cancellation)
            }
        } else {
            checked(cancellation) { platform.appendToFocused(text, cancellation) }
        }
        if (!result.accepted) {
            throw DriverException("DRIVER_ERROR", "type action not accepted by target")
        }
        return mutate(result.read, cancellation)
    }

    private fun swipe(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val direction = params.opt("direction")
        if (direction !is String || direction !in setOf("up", "down", "left", "right")) {
            throw DriverException("BAD_REQUEST", "direction up|down|left|right")
        }
        if (params.has("ref")) {
            val result = checked(cancellation) {
                platform.performNodeAction(ledger.lookup(params.opt("ref")), NodeAction.Scroll(direction), cancellation)
            }
            if (!result.accepted) {
                val resolved = ResolvedNode(result.node, result.read)
                checked(cancellation) {
                    platform.dispatchGesture(
                        checkedRefGesture(resolved, Gestures.swipe(result.node.identity.rawBounds, direction, 0.6)),
                    )
                }
            }
            return mutate(result.read, cancellation)
        }
        val live = checked(cancellation) { platform.readTree() }
        checked(cancellation) {
            platform.dispatchGesture(checkedRootGesture(Gestures.swipe(live.origin, direction, 0.5), live))
        }
        return mutate(live, cancellation)
    }

    private fun global(action: GlobalAction, cancellation: RequestCancellation): JSONObject {
        if (!checked(cancellation) { platform.performGlobal(action) }) {
            throw DriverException("DRIVER_ERROR", "${action.name.lowercase()} action refused")
        }
        return mutate(null, cancellation)
    }

    private fun center(params: JSONObject, cancellation: RequestCancellation): JSONObject {
        val which = params.opt("which")
        if (which !is String || which != "notification") {
            throw DriverException("BAD_REQUEST", "which notification")
        }
        return global(GlobalAction.NOTIFICATIONS, cancellation)
    }

    private fun screenshot(cancellation: RequestCancellation): JSONObject = JSONObject().put(
        "png_base64",
        java.util.Base64.getEncoder().encodeToString(checked(cancellation) { platform.screenshot() }),
    )
}
