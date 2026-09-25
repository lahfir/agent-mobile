import XCTest

// Private XCTest input paths: pointer events through XCPointerEventPath +
// XCSynthesizedEventRecord (the WebDriverAgent path, docs/research/11) and
// value writes through the accessibility client. Every selector's type
// encoding is checked before its IMP is cast, so a renamed or reshaped API
// throws DRIVER_ERROR instead of crashing the runner.
enum Events {
    static func miss() -> DrvError {
        DrvError(code: "DRIVER_ERROR", msg: "private XCTest event API missing on iOS \(UIDevice.current.systemVersion); rebuild the driver with the current Xcode")
    }

    /// Press at `p`, lift after `hold` seconds.
    static func press(_ p: CGPoint, hold: Double) throws {
        let path = try touch(at: p)
        try lift(path, at: hold)
        try send(path)
    }

    /// Press at `a`, stay `hold` seconds, move to `b` over `duration`, lift.
    static func drag(from a: CGPoint, to b: CGPoint, hold: Double, duration: Double) throws {
        let path = try touch(at: a)
        if hold > 0 { try move(path, to: a, at: hold) }
        try move(path, to: b, at: hold + duration)
        try lift(path, at: hold + duration)
        try send(path)
    }

    /// Keystrokes into the focused element at `keysPerSecond`.
    static func type(_ text: String, keysPerSecond: UInt64) throws {
        let (c, blank): (AnyClass, AnyObject) = try blank("XCPointerEventPath")
        let (_, initSel, initImp) = try method(c, "initForTextInput", [], "@")
        let (_, typeSel, typeImp) = try method(c, "typeText:atOffset:typingSpeed:shouldRedact:", ["@", "d", "Q", "B"], "v")
        let path = unsafeBitCast(initImp, to: (@convention(c) (AnyObject, Selector) -> AnyObject).self)(blank, initSel)
        unsafeBitCast(typeImp, to: (@convention(c) (AnyObject, Selector, AnyObject, Double, UInt64, Bool) -> Void).self)(
            path, typeSel, text as NSString, 0, keysPerSecond, false)
        try send(path)
    }

    /// Write `value` into the element's accessibility value; true when the
    /// client accepts it (callers still verify by re-reading).
    static func setValue(_ value: String, on element: AnyObject) throws -> Bool {
        let device = XCUIDevice.shared
        guard device.responds(to: NSSelectorFromString("accessibilityInterface")),
              let client = device.value(forKey: "accessibilityInterface") as AnyObject? else { throw miss() }
        let (_, sel, imp) = try method(object_getClass(client), "setAttribute:value:element:outError:", ["@", "@", "@", "^@"], "B")
        var err: NSError?
        return unsafeBitCast(imp, to: (@convention(c) (AnyObject, Selector, AnyObject, AnyObject, AnyObject, UnsafeMutablePointer<NSError?>) -> Bool).self)(
            client, sel, "XC_kAXXCAttributeValue" as NSString, value as NSString, element, &err)
    }

    /// An allocated, not yet initialised instance of a private class.
    private static func blank(_ name: String) throws -> (AnyClass, AnyObject) {
        let (cls, sel, imp): (AnyClass, Selector, IMP) = try method(NSClassFromString(name), "alloc", [], "@", classMethod: true)
        return (cls, unsafeBitCast(imp, to: (@convention(c) (AnyClass, Selector) -> AnyObject).self)(cls, sel))
    }

    private static func method(_ cls: AnyClass?, _ name: String, _ args: [String], _ ret: String,
                               classMethod: Bool = false) throws -> (AnyClass, Selector, IMP) {
        let sel = NSSelectorFromString(name)
        guard let cls, let m = classMethod ? class_getClassMethod(cls, sel) : class_getInstanceMethod(cls, sel),
              Runtime.sig(m, args, ret) else { throw miss() }
        return (cls, sel, method_getImplementation(m))
    }

    private static func touch(at p: CGPoint) throws -> AnyObject {
        let (c, blank): (AnyClass, AnyObject) = try blank("XCPointerEventPath")
        let (_, sel, imp) = try method(c, "initForTouchAtPoint:offset:", ["{CGPoint=dd}", "d"], "@")
        return unsafeBitCast(imp, to: (@convention(c) (AnyObject, Selector, CGPoint, Double) -> AnyObject).self)(blank, sel, p, 0)
    }

    private static func move(_ path: AnyObject, to p: CGPoint, at t: Double) throws {
        let (_, sel, imp) = try method(object_getClass(path), "moveToPoint:atOffset:", ["{CGPoint=dd}", "d"], "v")
        unsafeBitCast(imp, to: (@convention(c) (AnyObject, Selector, CGPoint, Double) -> Void).self)(path, sel, p, t)
    }

    private static func lift(_ path: AnyObject, at t: Double) throws {
        let (_, sel, imp) = try method(object_getClass(path), "liftUpAtOffset:", ["d"], "v")
        unsafeBitCast(imp, to: (@convention(c) (AnyObject, Selector, Double) -> Void).self)(path, sel, t)
    }

    private static func send(_ path: AnyObject) throws {
        let (c, blank): (AnyClass, AnyObject) = try blank("XCSynthesizedEventRecord")
        let (_, initSel, initImp) = try method(c, "initWithName:interfaceOrientation:", ["@", "q"], "@")
        let (_, addSel, addImp) = try method(c, "addPointerEventPath:", ["@"], "v")
        let (_, synthSel, synthImp) = try method(c, "synthesizeWithError:", ["^@"], "B")
        let orientation: Int = switch XCUIDevice.shared.orientation {
        case .landscapeLeft: 3
        case .landscapeRight: 4
        case .portraitUpsideDown: 2
        default: 1
        }
        let record = unsafeBitCast(initImp, to: (@convention(c) (AnyObject, Selector, AnyObject, Int) -> AnyObject).self)(
            blank, initSel, "agent-mobile" as NSString, orientation)
        unsafeBitCast(addImp, to: (@convention(c) (AnyObject, Selector, AnyObject) -> Void).self)(record, addSel, path)
        var err: NSError?
        guard unsafeBitCast(synthImp, to: (@convention(c) (AnyObject, Selector, UnsafeMutablePointer<NSError?>) -> Bool).self)(record, synthSel, &err) else {
            throw DrvError(code: "DRIVER_ERROR", msg: "event synthesis failed: \(err?.localizedDescription ?? "no detail")")
        }
    }
}
