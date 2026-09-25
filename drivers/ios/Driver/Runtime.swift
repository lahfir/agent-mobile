import Foundation

// ObjC runtime signature helpers used to verify a private-API method's shape
// before its IMP is bitcast, plus the quiescence-wait swizzle that relies on
// them. Kept apart from Driver so both it and Events can call in.
enum Runtime {
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
                    guard let imp = Runtime.noopImp(m) else { continue }
                    method_setImplementation(m, imp)
                    patched += 1
                } else if name.hasPrefix("_notifyWhen"), name.hasSuffix("Idle:"),
                          Runtime.argc(m) == 1, Runtime.encoding(of: m, at: 2) == "@?" {
                    let fireNow: @convention(block) (AnyObject, AnyObject) -> Void = { _, cb in
                        (unsafeBitCast(cb, to: (@convention(block) () -> Void).self))()
                    }
                    method_setImplementation(m, imp_implementationWithBlock(fireNow))
                    patched += 1
                } else if (name.hasPrefix("shouldSkip") && name.hasSuffix("Quiescence"))
                            || name == "isQuiescent" || name == "eventLoopHasIdled",
                          Runtime.argc(m) == 0, Runtime.returnEncoding(m) == "B" {
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
}
