import XCTest
import UIKit

// agent-mobile driver. One never-ending test method hosts a tiny HTTP server
// (WebDriverAgent shape). XCUITest is the hidden mechanism; the agent only sees HTTP.
// Upgrade path: JSON tree -> agent-desktop CLI wrapper; bearer token already required.

struct DrvError: Error { let code: String; let msg: String }

struct Ident { let type: XCUIElement.ElementType; let id: String; let label: String; let frame: CGRect }

final class Driver {
    var bundle = "com.apple.springboard"
    var snapId = ""
    var refs: [String: Ident] = [:]
    var seq = 0
    var lastRead: (h: Int, at: Date)?
    var appCache: [String: XCUIApplication] = [:]

    func app() -> XCUIApplication { app(bundle) }

    func app(_ bundleId: String) -> XCUIApplication {
        if let a = appCache[bundleId] { return a }
        let a = XCUIApplication(bundleIdentifier: bundleId)
        let setter = NSSelectorFromString("setIdleAnimationWaitEnabled:")
        if a.responds(to: setter) { a.setValue(false, forKey: "idleAnimationWaitEnabled") }
        appCache[bundleId] = a
        return a
    }

    func newSnapId() -> String {
        String(UInt64.random(in: 0...UInt64.max), radix: 36).prefix(8).description
    }

    // XCTest waits for app idle inside every interaction — unbounded, ~1 s in
    // practice. WebDriverAgent disables it by swizzling the private wait on
    // XCUIApplicationProcess (docs/research/11 §1.2); our own bounded settle
    // check replaces it. Guarded: a renamed method degrades to the stock
    // wait instead of a crash.
    static func killQuiescenceWait() {
        var patched = 0
        for cname in ["XCUIApplicationProcess", "XCUIApplication"] {
            guard let cls = NSClassFromString(cname) else { continue }
            var count: UInt32 = 0
            guard let list = class_copyMethodList(cls, &count) else { continue }
            for i in 0..<Int(count) {
                let m = list[i]
                let name = NSStringFromSelector(method_getName(m))
                if name.contains("waitForQuiescence") || name.hasPrefix("_initiateQuiescenceChecks") {
                    guard let imp = Driver.noopImp(m) else { continue }
                    method_setImplementation(m, imp)
                    patched += 1
                } else if name.hasPrefix("_notifyWhen"), name.hasSuffix("Idle:"),
                          Driver.argc(m) == 1, Driver.encoding(of: m, at: 2) == "@?" {
                    let fireNow: @convention(block) (AnyObject, AnyObject) -> Void = { _, cb in
                        (unsafeBitCast(cb, to: (@convention(block) () -> Void).self))()
                    }
                    method_setImplementation(m, imp_implementationWithBlock(fireNow))
                    patched += 1
                } else if (name.hasPrefix("shouldSkip") && name.hasSuffix("Quiescence"))
                            || name == "isQuiescent" || name == "eventLoopHasIdled",
                          Driver.argc(m) == 0, Driver.returnEncoding(m) == "B" {
                    let yes: @convention(block) (AnyObject) -> Bool = { _ in true }
                    method_setImplementation(m, imp_implementationWithBlock(yes))
                    patched += 1
                }
            }
            free(list)
        }
        NSLog("agent-mobile: quiescence wait disabled on %d selector(s)", patched)
        // testmanagerd confirms each synthesized event after a fixed interval;
        // zero it so injections return as soon as the event lands.
        if let cls = NSClassFromString("XCTRunnerDaemonSession") {
            let sessSel = NSSelectorFromString("sharedSession")
            let confSel = NSSelectorFromString("setImplicitEventConfirmationIntervalForCurrentContext:")
            if let sm = class_getClassMethod(cls, sessSel),
               let cm = class_getInstanceMethod(cls, confSel) {
                typealias ObjOptFn = @convention(c) (AnyObject, Selector) -> AnyObject?
                typealias VoidDblFn = @convention(c) (AnyObject, Selector, Double) -> Void
                if let sess = unsafeBitCast(method_getImplementation(sm), to: ObjOptFn.self)(cls, sessSel) {
                    unsafeBitCast(method_getImplementation(cm), to: VoidDblFn.self)(sess, confSel, 0)
                    NSLog("agent-mobile: implicit event confirmation interval zeroed")
                }
            }
        }
    }

    /// A no-op IMP for a `void` method whose args are all BOOLs. The block
    /// trampoline marshals arguments by signature, so arity AND the return
    /// type must match or the call forwards into a crash.
    static func noopImp(_ m: Method) -> IMP? {
        guard returnEncoding(m) == "v",
              (0..<argc(m)).allSatisfy({ encoding(of: m, at: $0 + 2) == "B" }) else { return nil }
        switch argc(m) {
        case 0:
            let b: @convention(block) (AnyObject) -> Void = { _ in }
            return imp_implementationWithBlock(b)
        case 1:
            let b: @convention(block) (AnyObject, Bool) -> Void = { _, _ in }
            return imp_implementationWithBlock(b)
        case 2:
            let b: @convention(block) (AnyObject, Bool, Bool) -> Void = { _, _, _ in }
            return imp_implementationWithBlock(b)
        case 3:
            let b: @convention(block) (AnyObject, Bool, Bool, Bool) -> Void = { _, _, _, _ in }
            return imp_implementationWithBlock(b)
        default:
            return nil
        }
    }

    static func argc(_ m: Method) -> Int { Int(method_getNumberOfArguments(m)) - 2 }

    static func encoding(of m: Method, at i: Int) -> String {
        guard let p = method_copyArgumentType(m, UInt32(i)) else { return "?" }
        defer { free(p) }
        return String(cString: p)
    }

    static func returnEncoding(_ m: Method) -> String {
        let p = method_copyReturnType(m)
        defer { free(p) }
        return String(cString: p)
    }

    /// Does `m` carry exactly `args` (in order, after self/_cmd) and return
    /// `ret`? Private-API calls bitcast IMPs to fixed signatures; verifying
    /// the encoding first turns drift into a graceful fallback.
    static func sig(_ m: Method, _ args: [String], _ ret: String) -> Bool {
        argc(m) == args.count
            && returnEncoding(m) == ret
            && args.enumerated().allSatisfy { encoding(of: m, at: $0.offset + 2) == $0.element }
    }

    func handle(_ cmd: String, _ p: [String: Any]) throws -> [String: Any] {
        switch cmd {
        case "status":
            return ["app": bundle, "snapshot_id": snapId, "device": UIDevice.current.name, "os": UIDevice.current.systemVersion]
        case "launch":
            let b = try str(p, "bundle_id")
            let a = app(b)
            a.launch()
            guard a.state == .runningForeground else {
                throw DrvError(code: "DRIVER_ERROR", msg: "launch of \(b) did not reach the foreground")
            }
            bundle = b
            return try settledSnapshot()
        case "activate":
            let b = try str(p, "bundle_id")
            app(b).activate()
            bundle = b
            return try settledSnapshot()
        case "terminate":
            guard bundle != "com.apple.springboard" else {
                throw DrvError(code: "BAD_REQUEST", msg: "no active app to terminate")
            }
            let old = bundle
            app().terminate()
            bundle = "com.apple.springboard"
            snapId = newSnapId(); refs = [:]; seq = 0; lastRead = nil
            return ["terminated": old]
        case "snapshot":
            let target = p["app"] == nil ? bundle : try str(p, "app")
            let out = try settledSnapshot(target: target)
            bundle = target
            return out
        case "tap":
            let (pt, base) = try resolvePoint(p)
            tapPoint(pt)
            return try settledSnapshot(baseline: base)
        case "hold":
            let dur: Double
            if let raw = p["duration"] {
                guard let d = raw as? Double else {
                    throw DrvError(code: "BAD_REQUEST", msg: "duration must be a number")
                }
                dur = d
            } else {
                dur = 1.0
            }
            // Backstop cap: press + settle must fit the CLI wire budget; the
            // CLI also sizes its own timeout from duration.
            guard dur > 0, dur.isFinite, dur <= 10 else {
                throw DrvError(code: "BAD_REQUEST", msg: "duration 0 < d <= 10")
            }
            let (pt, hbase) = try resolvePoint(p)
            guard try pressRecord(pt, liftDelay: dur) else {
                throw DrvError(code: "DRIVER_ERROR", msg: "pointer-event synthesis returned false")
            }
            return try settledSnapshot(baseline: hbase)
        case "doubletap":
            let (pt, dbase) = try resolvePoint(p)
            tapPoint(pt)
            tapPoint(pt)
            return try settledSnapshot(baseline: dbase)
        case "pinch":
            guard let scale = p["scale"] as? Double, scale > 0, scale.isFinite, abs(scale - 1) >= 0.01 else {
                throw DrvError(code: "BAD_REQUEST", msg: "scale positive, finite, |scale-1| >= 0.01")
            }
            let velocity: Double
            if let raw = p["velocity"] {
                guard let v = raw as? Double, v.isFinite else {
                    throw DrvError(code: "BAD_REQUEST", msg: "velocity finite")
                }
                velocity = v
            } else {
                velocity = scale > 1 ? 1.0 : -1.0
            }
            let (el, base) = try element(p["ref"])
            try gesture("pinch") { el.pinch(withScale: CGFloat(scale), velocity: CGFloat(velocity)) }
            return try settledSnapshot(baseline: base)
        case "twofinger":
            let (el, base) = try element(p["ref"])
            try gesture("twofinger") { el.twoFingerTap() }
            return try settledSnapshot(baseline: base)
        case "back":
            let f = app().frame
            let y = f.minY + f.height * 0.4
            edgeDrag(from: CGPoint(x: f.minX + f.width * 0.03, y: y),
                     to: CGPoint(x: f.minX + f.width * 0.8, y: y))
            return try settledSnapshot()
        case "center":
            guard let which = p["which"] as? String, which == "notification" else {
                throw DrvError(code: "BAD_REQUEST", msg: "which notification")
            }
            bundle = "com.apple.springboard"
            let sf = app().frame
            edgeDrag(from: CGPoint(x: sf.minX + sf.width * 0.15, y: sf.minY + sf.height * 0.02),
                     to: CGPoint(x: sf.minX + sf.width * 0.15, y: sf.minY + sf.height * 0.7), hold: 0)
            return try settledSnapshot()
        case "type":
            let text = try str(p, "text")
            var base: (h: Int, at: Date)? = nil
            if p["ref"] != nil {
                let (f, rh) = try resolve(p["ref"])
                let pt = CGPoint(x: f.midX, y: f.midY)
                if !fastTap(pt) { point(pt).tap() }
                base = rh
            }
            app().typeText(text); return try settledSnapshot(baseline: base)
        case "swipe":
            let dir = try str(p, "direction")
            guard ["up", "down", "left", "right"].contains(dir) else {
                throw DrvError(code: "BAD_REQUEST", msg: "direction up|down|left|right")
            }
            if p["ref"] != nil {
                let (f, h) = try resolve(p["ref"]); flick(dir, from: f); return try settledSnapshot(baseline: h)
            }
            let el = app()
            switch dir {
            case "up": el.swipeUp(); case "down": el.swipeDown(); case "left": el.swipeLeft(); default: el.swipeRight()
            }
            return try settledSnapshot()
        case "home":
            XCUIDevice.shared.press(.home); bundle = "com.apple.springboard"; return try settledSnapshot()
        case "screenshot":
            return ["png_base64": XCUIScreen.main.screenshot().pngRepresentation.base64EncodedString()]
        default:
            throw DrvError(code: "UNKNOWN_COMMAND", msg: cmd)
        }
    }

    // Absolute point in the app frame -> a tap-able coordinate; no query.
    func point(_ p: CGPoint) -> XCUICoordinate {
        app().coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: p.x, dy: p.y))
    }

    // One tap through the fast record path with stock-tap fallback.
    func tapPoint(_ pt: CGPoint) {
        if !fastTap(pt) { point(pt).tap() }
    }

    // XCTest signals an unaddressable element by RAISING NSException, which
    // Swift do/catch cannot intercept: the raise aborts testServe and kills
    // the driver for every later verb (seen live on a Maps widget whose
    // coordinates XCTest computed as {inf, inf}). The two element-only
    // verbs route through the ObjC catcher so a bad element reports
    // DRIVER_ERROR and the driver stays up.
    func gesture(_ what: String, _ block: @escaping () -> Void) throws {
        if let msg = AMGestureCatch.runGesture(block) {
            throw DrvError(code: "DRIVER_ERROR", msg: "\(what): \(msg)")
        }
    }

    // Shared ref-or-coordinates resolution for the point verbs (tap,
    // doubletap, hold). An explicit x/y pair must arrive together; a body
    // with only one names neither a point nor (yet) a ref, so it fails here
    // instead of falling through to a misleading "ref required".
    func resolvePoint(_ p: [String: Any]) throws -> (pt: CGPoint, base: (h: Int, at: Date)?) {
        if let x = p["x"] as? Double, let y = p["y"] as? Double {
            return (CGPoint(x: x, y: y), nil)
        }
        if p["x"] != nil || p["y"] != nil {
            throw DrvError(code: "BAD_REQUEST", msg: "x and y must be sent together")
        }
        let (f, base) = try resolve(p["ref"])
        return (CGPoint(x: f.midX, y: f.midY), base)
    }

    // A tap via XCPointerEventPath + XCSynthesizedEventRecord — the WebDriverAgent
    // event path (docs/research/11). `XCUICoordinate.tap()` pays an element
    // resolution plus XCTest's event plumbing (~480 ms here); the record
    // synthesizes straight through testmanagerd. Every selector is verified at
    // runtime; any miss falls back to the stock coordinate tap.
    func fastTap(_ p: CGPoint) -> Bool {
        (try? pressRecord(p, liftDelay: 0.05)) ?? false
    }

    // The shared pointer-event record builder behind `fastTap`. A delayed
    // `liftUpAtOffset` forms a press-and-hold; XCTest `element.press` stalls
    // 60–100 s on some targets, so `hold` has no element-press fallback — a
    // signature miss throws DRIVER_ERROR naming the runtime instead of
    // falling through to a stall.
    static func pressMiss() -> DrvError {
        DrvError(code: "DRIVER_ERROR", msg: "pointer-event record signatures missing on iOS \(UIDevice.current.systemVersion); rebuild the driver with the current Xcode")
    }

    func pressRecord(_ p: CGPoint, liftDelay: Double) throws -> Bool {
        guard let pathCls = NSClassFromString("XCPointerEventPath"),
              let recCls = NSClassFromString("XCSynthesizedEventRecord") else { throw Driver.pressMiss() }
        let allocSel = NSSelectorFromString("alloc")
        let initSel = NSSelectorFromString("initForTouchAtPoint:offset:")
        let liftSel = NSSelectorFromString("liftUpAtOffset:")
        let recSel = NSSelectorFromString("initWithName:interfaceOrientation:")
        let addSel = NSSelectorFromString("addPointerEventPath:")
        let synthSel = NSSelectorFromString("synthesizeWithError:")
        guard let allocM = class_getClassMethod(pathCls, allocSel), Driver.sig(allocM, [], "@"),
              let recAllocM = class_getClassMethod(recCls, allocSel), Driver.sig(recAllocM, [], "@"),
              let initM = class_getInstanceMethod(pathCls, initSel), Driver.sig(initM, ["{CGPoint=dd}", "d"], "@"),
              let liftM = class_getInstanceMethod(pathCls, liftSel), Driver.sig(liftM, ["d"], "v"),
              let recM = class_getInstanceMethod(recCls, recSel), Driver.sig(recM, ["@", "q"], "@"),
              let addM = class_getInstanceMethod(recCls, addSel), Driver.sig(addM, ["@"], "v"),
              let synthM = class_getInstanceMethod(recCls, synthSel), Driver.sig(synthM, ["^@"], "B") else { throw Driver.pressMiss() }
        typealias ObjFn = @convention(c) (AnyObject, Selector) -> AnyObject
        typealias TouchFn = @convention(c) (AnyObject, Selector, CGPoint, Double) -> AnyObject
        typealias VoidDblFn = @convention(c) (AnyObject, Selector, Double) -> Void
        typealias NameOriFn = @convention(c) (AnyObject, Selector, AnyObject, Int) -> AnyObject
        typealias VoidObjFn = @convention(c) (AnyObject, Selector, AnyObject) -> Void
        typealias SynthFn = @convention(c) (AnyObject, Selector, UnsafeMutablePointer<NSError?>) -> Bool
        let path = unsafeBitCast(method_getImplementation(initM), to: TouchFn.self)(
            unsafeBitCast(method_getImplementation(allocM), to: ObjFn.self)(pathCls, allocSel),
            initSel, p, 0)
        unsafeBitCast(method_getImplementation(liftM), to: VoidDblFn.self)(path, liftSel, liftDelay)
        let ori: Int = switch XCUIDevice.shared.orientation {
        case .landscapeLeft: 3
        case .landscapeRight: 4
        case .portraitUpsideDown: 2
        default: 1
        }
        let rec = unsafeBitCast(method_getImplementation(recM), to: NameOriFn.self)(
            unsafeBitCast(method_getImplementation(recAllocM), to: ObjFn.self)(recCls, allocSel),
            recSel, "agent-mobile" as NSString, ori)
        unsafeBitCast(method_getImplementation(addM), to: VoidObjFn.self)(rec, addSel, path)
        var err: NSError?
        return unsafeBitCast(method_getImplementation(synthM), to: SynthFn.self)(rec, synthSel, &err)
    }

    // A ref-scoped swipe as a press+drag across the element's frame.
    func flick(_ dir: String, from f: CGRect) {
        var dx = 0.0, dy = 0.0
        switch dir {
        case "up": dy = -f.height * 0.6
        case "down": dy = f.height * 0.6
        case "left": dx = -f.width * 0.6
        default: dx = f.width * 0.6
        }
        point(CGPoint(x: f.midX, y: f.midY))
            .press(forDuration: 0.05, thenDragTo: point(CGPoint(x: f.midX + dx, y: f.midY + dy)))
    }

    // One press-drag primitive for the KTD3 edge verbs (back, center):
    // endpoints in app-frame coordinates. Back uses the probed 0.1 s hold;
    // center passes 0 so status-bar recognizers cannot claim the touch.
    // Same coordinate helpers as flick.
    func edgeDrag(from a: CGPoint, to b: CGPoint, hold: TimeInterval = 0.1) {
        point(a).press(forDuration: hold, thenDragTo: point(b))
    }

    func str(_ p: [String: Any], _ k: String) throws -> String {
        guard let v = p[k] as? String, !v.isEmpty else { throw DrvError(code: "BAD_REQUEST", msg: "\(k) required") }
        return v
    }

    // Per-snapshot qualified refs; re-resolve on every action and fail loudly.
    // One fresh tree read is walked in-process for (type, identifier, label,
    // frame ±1 pt) instead of a predicate query that XCTest evaluates twice;
    // the caller acts on the matched frame's coordinates. The read's hash and
    // timestamp are returned so the settle check can count it as the first
    // consecutive read only when it is old enough to prove an interval.
    // Shared ref-ledger preamble for resolve() and element(): string cast,
    // snapshot-id match, ledger hit. Matching stays per-verb below so the
    // STALE_REF contract cannot drift between copies.
    func lookupIdent(_ any: Any?) throws -> (ref: String, id: Ident) {
        guard let ref = any as? String else { throw DrvError(code: "BAD_REQUEST", msg: "ref required") }
        let parts = ref.split(separator: ":")
        guard parts.count == 2, String(parts[0].dropFirst()) == snapId else {
            throw DrvError(code: "STALE_REF", msg: "current snapshot is @\(snapId); ref \(ref) is from another snapshot; re-snapshot")
        }
        guard let id = refs[ref] else { throw DrvError(code: "STALE_REF", msg: "unknown ref \(ref)") }
        return (ref, id)
    }

    func resolve(_ any: Any?) throws -> (frame: CGRect, read: (h: Int, at: Date)) {
        let (ref, id) = try lookupIdent(any)
        let snap = try app().snapshot()
        let at = Date()
        var n = 0; var frame = CGRect.zero
        func walk(_ node: XCUIElementSnapshot) {
            if node.elementType == id.type, Driver.snapString(node, "identifier") == id.id, Driver.snapString(node, "label") == id.label,
               Driver.close(node.frame, id.frame) { n += 1; frame = node.frame }
            node.children.forEach(walk)
        }
        walk(snap)
        if n == 0 { throw DrvError(code: "STALE_REF", msg: "\(ref) no longer matches a live element; re-snapshot") }
        if n > 1 { throw DrvError(code: "AMBIGUOUS_TARGET", msg: "\(ref) matches \(n) live elements; re-snapshot") }
        return (frame, (hash(snap), at))
    }

    // Ref-to-element twin of resolve() for the verbs XCTest only offers on
    // XCUIElement (pinch, twoFingerTap). Same ledger, same
    // STALE_REF/AMBIGUOUS_TARGET contract, matched on type + identifier +
    // label + frame with no isHittable reads (KTD2). The returned read seeds
    // the settle baseline like resolve()'s does.
    func element(_ any: Any?) throws -> (el: XCUIElement, read: (h: Int, at: Date)) {
        let (ref, id) = try lookupIdent(any)
        let snap = try app().snapshot()
        let at = Date()
        // Filter identifier and label inside XCTest so only candidates pay
        // IPC materialization; frame stays in Swift (not predicate-safe).
        // Fall back to the full walk: predicate semantics can miss elements
        // XCTest reports differently (notably empty identifiers).
        let pred = NSPredicate(format: "identifier == %@ AND label == %@", id.id, id.label)
        var found = app().descendants(matching: id.type).matching(pred).allElementsBoundByIndex.filter {
            Driver.close($0.frame, id.frame)
        }
        if found.isEmpty {
            found = app().descendants(matching: id.type).allElementsBoundByIndex.filter {
                Driver.snapString($0, "identifier") == id.id && Driver.snapString($0, "label") == id.label && Driver.close($0.frame, id.frame)
            }
        }
        if found.isEmpty { throw DrvError(code: "STALE_REF", msg: "\(ref) no longer matches a live element; re-snapshot") }
        if found.count > 1 { throw DrvError(code: "AMBIGUOUS_TARGET", msg: "\(ref) matches \(found.count) live elements; re-snapshot") }
        return (found[0], (hash(snap), at))
    }

    static func close(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) <= 1 && abs(a.minY - b.minY) <= 1 && abs(a.width - b.width) <= 1 && abs(a.height - b.height) <= 1
    }

    // System trees can expose non-finite or out-of-range frames that trap a
    // raw Int(Double). qi quantizes for text/hash; qd preserves fractions
    // for the JSON bounds dict (JSONSerialization rejects NaN/Inf).
    // Clamp band is one no real frame reaches.
    static func qi(_ v: Double) -> Int {
        guard v.isFinite else { return 0 }
        return Int(max(-1_000_000, min(1_000_000, v.rounded())))
    }

    static func qd(_ v: Double) -> Double {
        guard v.isFinite else { return 0 }
        return max(-1_000_000, min(1_000_000, v))
    }

    // Cover-sheet nodes violate nullability the way they violate finite
    // frames: a nil label/identifier/title against the non-optional type
    // traps the process (SIGTRAP) at the typed access, killing the driver
    // mid-snapshot. KVC returns Any?, so this bridge is nil-safe.
    static func snapString(_ o: Any, _ key: String) -> String {
        guard let ns = o as? NSObject else {
            NSLog("agent-mobile: non-NSObject snapshot type=%@", String(describing: type(of: o)))
            return ""
        }
        if let v = ns.value(forKey: key) as? String { return v }
        NSLog("agent-mobile: nil snapshot string key=%@ type=%@", key, String(describing: type(of: o)))
        return ""
    }

    // Two-read tree-hash idle check with a finite cap (Apple's own quiescence
    // wait is unbounded). Reads are spaced by at least `gap` — two matching
    // hashes taken microseconds apart prove nothing. `baseline` donates an
    // already-taken read (the action's own resolve read, or the last served
    // snapshot) as the first of the pair, but only counts when it is at
    // least `gap` old; a fresher donation just becomes the first poll read.
    func settledSnapshot(timeout: TimeInterval = 3, baseline: (h: Int, at: Date)? = nil, target: String? = nil) throws -> [String: Any] {
        let gap: TimeInterval = 0.15
        let a = app(target ?? bundle)
        let started = Date(); let deadline = started.addingTimeInterval(timeout)
        var snap = try a.snapshot(); var reads = 1; var h = hash(snap); var at = Date()
        var settled = (baseline ?? lastRead).map { $0.h == h && at.timeIntervalSince($0.at) >= gap } ?? false
        while !settled, Date() < deadline {
            let wait = gap - Date().timeIntervalSince(at)
            if wait > 0 { usleep(UInt32(wait * 1_000_000)) }
            let s2 = try a.snapshot(); reads += 1
            let h2 = hash(s2); let at2 = Date(); snap = s2
            if h2 == h { settled = true }
            h = h2; at = at2
        }
        let settleMs = Int(Date().timeIntervalSince(started) * 1000)
        lastRead = (h, at)
        snapId = newSnapId()
        refs = [:]; seq = 0
        var lines: [String] = []
        let tree = build(snap, pdepth: 0, lines: &lines)
        return ["app": target ?? bundle, "snapshot_id": snapId, "ref_count": seq, "complete": true, "settled": settled,
                "reads": reads, "settle_ms": settleMs, "text": lines.joined(separator: "\n"), "tree": tree]
    }

    func hash(_ s: XCUIElementSnapshot) -> Int {
        var h = Hasher()
        func walk(_ n: XCUIElementSnapshot) {
            h.combine(n.elementType.rawValue); h.combine(Driver.qi(n.frame.minX)); h.combine(Driver.qi(n.frame.minY))
            h.combine(Driver.qi(n.frame.width)); h.combine(Driver.qi(n.frame.height)); h.combine(Driver.snapString(n, "label")); h.combine(Driver.snapString(n, "identifier"))
            h.combine(n.value.map { String(describing: $0) } ?? "")
            n.children.forEach(walk)
        }
        walk(s); return h.finalize()
    }

    func build(_ n: XCUIElementSnapshot, pdepth: Int, lines: inout [String]) -> [String: Any] {
        seq += 1
        let ref = "@\(snapId):e\(seq)"
        let label = Driver.snapString(n, "label")
        let identifier = Driver.snapString(n, "identifier")
        refs[ref] = Ident(type: n.elementType, id: identifier, label: label, frame: n.frame)
        let role = Driver.role(n.elementType)
        let title = Driver.snapString(n, "title")
        let name = !label.isEmpty ? label : (!title.isEmpty ? title : (n.placeholderValue ?? identifier))
        let value = n.value.map { String(describing: $0) } ?? ""
        var states: [String] = []
        if !n.isEnabled { states.append("disabled") }
        if n.isSelected { states.append("selected") }
        if n.hasFocus { states.append("focused") }
        let actions = Driver.actions(n.elementType)
        let f = n.frame
        let printed = !name.isEmpty || !value.isEmpty || !actions.isEmpty
        if printed {
            var line = String(repeating: "  ", count: pdepth) + "\(ref) \(role) \"\(name)\""
            if !value.isEmpty { line += " value=\"\(value)\"" }
            line += " at=\(Driver.qi(f.minX)),\(Driver.qi(f.minY)) size=\(Driver.qi(f.width))x\(Driver.qi(f.height))"
            if !states.isEmpty { line += " [\(states.joined(separator: ","))]" }
            lines.append(line)
        }
        var node: [String: Any] = ["role": role, "name": name, "value": value, "ref_id": ref, "states": states,
                                   "available_actions": actions,
                                   "bounds": ["x": Driver.qd(f.minX), "y": Driver.qd(f.minY), "width": Driver.qd(f.width), "height": Driver.qd(f.height)]]
        if !identifier.isEmpty { node["native_id"] = ["kind": "ax_identifier", "value": identifier] }
        node["children"] = n.children.map { build($0, pdepth: printed ? pdepth + 1 : pdepth, lines: &lines) }
        return node
    }

    static func role(_ t: XCUIElement.ElementType) -> String {
        switch t {
        case .application: return "application"; case .window: return "window"; case .other: return "group"
        case .button: return "button"; case .staticText: return "text"; case .textField: return "textfield"
        case .secureTextField: return "securetextfield"; case .textView: return "textview"; case .searchField: return "searchfield"
        case .image: return "image"; case .cell: return "cell"; case .table: return "table"; case .collectionView: return "collectionview"
        case .scrollView: return "scrollview"; case .navigationBar: return "navigationbar"; case .tabBar: return "tabbar"
        case .toolbar: return "toolbar"; case .`switch`: return "switch"; case .toggle: return "toggle"; case .slider: return "slider"
        case .picker: return "picker"; case .pickerWheel: return "pickerwheel"; case .datePicker: return "datepicker"
        case .segmentedControl: return "segmentedcontrol"; case .alert: return "alert"; case .sheet: return "sheet"
        case .keyboard: return "keyboard"; case .key: return "key"; case .link: return "link"; case .menu: return "menu"
        case .menuItem: return "menuitem"; case .checkBox: return "checkbox"; case .radioButton: return "radio"
        case .tab: return "tab"; case .tabGroup: return "tabgroup"; case .webView: return "webview"; case .stepper: return "stepper"
        case .popover: return "popover"; case .activityIndicator: return "activity"; case .progressIndicator: return "progress"
        case .pageIndicator: return "pageindicator"; case .statusBar: return "statusbar"; case .map: return "map"
        case .icon: return "icon"; case .dialog: return "dialog"
        default: return "type\(t.rawValue)"
        }
    }

    static func actions(_ t: XCUIElement.ElementType) -> [String] {
        switch t {
        case .textField, .secureTextField, .textView, .searchField: return ["Tap", "Type"]
        case .`switch`, .toggle, .checkBox: return ["Tap"]
        case .button, .cell, .link, .key, .menuItem, .tab, .radioButton, .stepper, .segmentedControl, .datePicker, .pickerWheel, .slider: return ["Tap"]
        case .scrollView, .table, .collectionView: return ["Swipe"]
        default: return []
        }
    }
}

struct Request { let method: String; let path: String; let headers: [String: String]; let body: Data }

final class HTTPServer {
    let port: UInt16
    let bindAddr: String
    let handler: (Request) -> (Int, [String: Any])
    init(port: UInt16, bindAddr: String, handler: @escaping (Request) -> (Int, [String: Any])) { self.port = port; self.bindAddr = bindAddr; self.handler = handler }

    func start() { Thread { self.loop() }.start() }

    func loop() {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr.s_addr = inet_addr(bindAddr)  // 127.0.0.1 on the simulator; 0.0.0.0 on a physical device so the host reaches it over Wi-Fi
        let rc = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
        guard rc == 0, listen(fd, 8) == 0 else {
            NSLog("agent-mobile: bind/listen failed errno=%d — exiting so the launcher reports a dead driver", errno)
            exit(1)
        }
        NSLog("agent-mobile: listening on %@:%d", bindAddr, Int(port))
        while true {
            let c = accept(fd, nil, nil)
            if c < 0 { continue }
            serve(c); close(c)
        }
    }

    func serve(_ c: Int32) {
        // One stalled client must not wedge the serial accept loop: bound the
        // socket's own reads and writes, and cap header and body sizes.
        var tv = timeval(tv_sec: 10, tv_usec: 0)
        setsockopt(c, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(c, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        let terminator = Data("\r\n\r\n".utf8)
        var buf = Data(); var chunk = [UInt8](repeating: 0, count: 65536)
        var headerEnd: Range<Data.Index>? = nil
        while headerEnd == nil {
            if buf.count > 1_048_576 { return }
            let n = read(c, &chunk, chunk.count); if n <= 0 { return }
            buf.append(chunk, count: n); headerEnd = buf.range(of: terminator)
        }
        let head = String(decoding: buf[..<headerEnd!.lowerBound], as: UTF8.self)
        var lines = head.components(separatedBy: "\r\n")
        let reqLine = lines.removeFirst().split(separator: " ")
        guard reqLine.count >= 2 else { return }
        var headers: [String: String] = [:]
        for l in lines { if let i = l.firstIndex(of: ":") { headers[l[..<i].lowercased()] = l[l.index(after: i)...].trimmingCharacters(in: .whitespaces) } }
        let len = Int(headers["content-length"] ?? "0") ?? 0
        guard len >= 0, len <= 16_777_216 else { return }
        var body = Data(buf[headerEnd!.upperBound...])
        while body.count < len {
            let n = read(c, &chunk, chunk.count); if n <= 0 { return }
            body.append(chunk, count: n)
        }
        let (status, obj) = handler(Request(method: String(reqLine[0]), path: String(reqLine[1]), headers: headers, body: body))
        var payload: Data; var ctype = "application/json"
        if headers["accept"] == "text/plain", let d = obj["data"] as? [String: Any], let t = d["text"] as? String {
            let hdr = "app=\(d["app"] ?? "") snapshot=@\(d["snapshot_id"] ?? "") refs=\(d["ref_count"] ?? 0) settled=\(d["settled"] ?? "") reads=\(d["reads"] ?? "") elapsed_ms=\(obj["elapsed_ms"] ?? "")\n"
            payload = Data((hdr + t + "\n").utf8); ctype = "text/plain"
        } else {
            payload = (try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys])) ?? Data("{}".utf8)
        }
        var resp = Data("HTTP/1.1 \(status) \(status == 200 ? "OK" : "Error")\r\nContent-Type: \(ctype)\r\nContent-Length: \(payload.count)\r\nConnection: close\r\n\r\n".utf8)
        resp.append(payload)
        resp.withUnsafeBytes { p in
            var off = 0
            while off < resp.count { let n = write(c, p.baseAddress! + off, resp.count - off); if n <= 0 { break }; off += n }
        }
    }
}

final class AgentMobileServer: XCTestCase {
    static let drv = Driver()
    static let protocolVersion = "1"

    func testServe() {
        continueAfterFailure = true
        signal(SIGPIPE, SIG_IGN)
        Driver.killQuiescenceWait()
        let env = ProcessInfo.processInfo.environment
        let port = UInt16(env["AGENT_MOBILE_PORT"] ?? "") ?? 8770
        let token = env["AGENT_MOBILE_TOKEN"] ?? ""
        let bindAddr = env["AGENT_MOBILE_BIND"] ?? "127.0.0.1"
        let srv = HTTPServer(port: port, bindAddr: bindAddr) { req in
            let cmd = String(req.path.split(separator: "?").first ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let t0 = Date()
            let elapsed = { Int(Date().timeIntervalSince(t0) * 1000) }
            if token.isEmpty || req.headers["authorization"] != "Bearer \(token)" {
                return (401, ["version": AgentMobileServer.protocolVersion, "ok": false, "error": ["code": "UNAUTHORIZED", "message": "Authorization: Bearer <AGENT_MOBILE_TOKEN> required"]])
            }
            if req.method != "POST" {
                return (405, ["version": AgentMobileServer.protocolVersion, "ok": false, "command": cmd, "elapsed_ms": elapsed(), "error": ["code": "BAD_REQUEST", "message": "verbs are POST only"]])
            }
            if req.headers["x-agent-mobile-version"] != AgentMobileServer.protocolVersion {
                return (409, ["version": AgentMobileServer.protocolVersion, "ok": false, "command": cmd, "elapsed_ms": elapsed(), "error": ["code": "BAD_REQUEST", "message": "X-Agent-Mobile-Version must be 1"]])
            }
            let params = (try? JSONSerialization.jsonObject(with: req.body)) as? [String: Any] ?? [:]
            return DispatchQueue.main.sync {
                do {
                    let data = try AgentMobileServer.drv.handle(cmd, params)
                    return (200, ["version": AgentMobileServer.protocolVersion, "ok": true, "command": cmd, "elapsed_ms": elapsed(), "data": data])
                } catch let e as DrvError {
                    return (409, ["version": AgentMobileServer.protocolVersion, "ok": false, "command": cmd, "elapsed_ms": elapsed(), "error": ["code": e.code, "message": e.msg]])
                } catch {
                    return (500, ["version": AgentMobileServer.protocolVersion, "ok": false, "command": cmd, "elapsed_ms": elapsed(), "error": ["code": "DRIVER_ERROR", "message": "\(error)"]])
                }
            }
        }
        #if targetEnvironment(simulator)
        XCUIDevice.shared.press(.home)
        #endif
        srv.start()
        while true { RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.2)) }
    }
}
