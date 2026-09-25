import XCTest
import UIKit

struct DrvError: Error { let code: String; let msg: String }

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
            try Events.press(pt, hold: dur)
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
            drag(from: CGPoint(x: f.minX + f.width * 0.03, y: y), to: CGPoint(x: f.minX + f.width * 0.8, y: y), hold: 0.1)
            return try settledSnapshot()
        case "center":
            guard let which = p["which"] as? String, which == "notification" else {
                throw DrvError(code: "BAD_REQUEST", msg: "which notification")
            }
            bundle = "com.apple.springboard"
            let sf = app().frame
            drag(from: CGPoint(x: sf.minX + sf.width * 0.15, y: sf.minY + sf.height * 0.02),
                 to: CGPoint(x: sf.minX + sf.width * 0.15, y: sf.minY + sf.height * 0.7), hold: 0)
            return try settledSnapshot()
        case "type":
            let text = try str(p, "text")
            guard p["ref"] != nil else {
                typeKeys(text)
                return try settledSnapshot()
            }
            let (node, base) = try resolve(p["ref"])
            if try !setValue(node, appending: text) {
                tapPoint(CGPoint(x: node.frame.midX, y: node.frame.midY))
                typeKeys(text)
            }
            return try settledSnapshot(baseline: base)
        case "swipe":
            let dir = try str(p, "direction")
            guard ["up", "down", "left", "right"].contains(dir) else {
                throw DrvError(code: "BAD_REQUEST", msg: "direction up|down|left|right")
            }
            if p["ref"] != nil {
                let (node, h) = try resolve(p["ref"]); flick(dir, across: node.frame, span: 0.6); return try settledSnapshot(baseline: h)
            }
            flick(dir, across: app().frame, span: 0.5)
            return try settledSnapshot()
        case "home":
            XCUIDevice.shared.press(.home); bundle = "com.apple.springboard"; return try settledSnapshot()
        case "screenshot":
            return ["png_base64": XCUIScreen.main.screenshot().pngRepresentation.base64EncodedString()]
        default:
            throw DrvError(code: "UNKNOWN_COMMAND", msg: cmd)
        }
    }

    func str(_ p: [String: Any], _ k: String) throws -> String {
        guard let v = p[k] as? String, !v.isEmpty else { throw DrvError(code: "BAD_REQUEST", msg: "\(k) required") }
        return v
    }
}
