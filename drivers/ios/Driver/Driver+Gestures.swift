import XCTest
import UIKit

extension Driver {
    // Absolute point in the app frame -> a tap-able coordinate; no query.
    func point(_ p: CGPoint) -> XCUICoordinate {
        app().coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: p.x, dy: p.y))
    }

    // A tap through the pointer-event path (~210 ms, vs ~480 ms for
    // `XCUICoordinate.tap()`, which stays as the fallback when the runtime
    // lacks the path).
    func tapPoint(_ pt: CGPoint) {
        if (try? Events.press(pt, hold: 0.05)) == nil { point(pt).tap() }
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
        let (node, base) = try resolve(p["ref"])
        return (CGPoint(x: node.frame.midX, y: node.frame.midY), base)
    }

    // Press-hold, then drag from a to b in `duration` seconds, as one pointer
    // path (~300 ms vs ~1.2 s for `press(forDuration:thenDragTo:)`, which
    // stays as the fallback when the runtime lacks the path.
    func drag(from a: CGPoint, to b: CGPoint, hold: TimeInterval, duration: TimeInterval = 0.15) {
        if (try? Events.drag(from: a, to: b, hold: hold, duration: duration)) == nil {
            point(a).press(forDuration: hold, thenDragTo: point(b))
        }
    }

    // A swipe through the centre of `f`, travelling `span` of its size.
    func flick(_ dir: String, across f: CGRect, span: Double) {
        let c = CGPoint(x: f.midX, y: f.midY)
        let dx = dir == "left" ? -f.width * span : dir == "right" ? f.width * span : 0
        let dy = dir == "up" ? -f.height * span : dir == "down" ? f.height * span : 0
        drag(from: CGPoint(x: c.x - dx / 2, y: c.y - dy / 2), to: CGPoint(x: c.x + dx / 2, y: c.y + dy / 2), hold: 0.05, duration: 0.1)
    }

    // Keystrokes through the pointer-event text path at 1000 keys/s; XCTest's
    // `typeText` is fixed at 60 keys/s (200 chars: ~0.3 s vs ~3.8 s).
    func typeKeys(_ text: String) {
        if (try? Events.type(text, keysPerSecond: 1000)) == nil { app().typeText(text) }
    }

    // Write a text field's value through the accessibility client: one IPC,
    // no keystrokes (200 chars in 12-40 ms; UIKit and SwiftUI bindings both
    // see it). The value lands only if a re-read shows it; secure fields and
    // any refusal return false so the caller falls back to keystrokes.
    func setValue(_ node: XCUIElementSnapshot, appending text: String) throws -> Bool {
        guard node.elementType != .secureTextField,
              let el = (node as AnyObject).value(forKey: "accessibilityElement") as AnyObject? else { return false }
        let current = node.value.map { String(describing: $0) } ?? ""
        let want = (current == node.placeholderValue ? "" : current) + text
        guard (try? Events.setValue(want, on: el)) == true else { return false }
        let id = Ident(type: node.elementType, id: Driver.snapString(node, "identifier"), label: Driver.snapString(node, "label"), frame: node.frame)
        var got: String?
        func walk(_ n: XCUIElementSnapshot) {
            if got == nil, n.elementType == id.type, Driver.snapString(n, "identifier") == id.id, Driver.close(n.frame, id.frame) {
                got = n.value.map { String(describing: $0) }
            }
            n.children.forEach(walk)
        }
        walk(try app().snapshot())
        return got == want
    }
}
