import XCTest
import UIKit

// ponytail: probe-grade driver. One never-ending test method hosts a tiny HTTP server
// (WebDriverAgent shape). XCUITest is the hidden mechanism; the agent only sees HTTP.
// Upgrade path: JSON tree -> agent-desktop CLI wrapper; bearer token already required.

struct DrvError: Error { let code: String; let msg: String }

struct Ident { let type: XCUIElement.ElementType; let id: String; let label: String; let frame: CGRect }

final class Driver {
    var bundle = "com.apple.springboard"
    var snapId = ""
    var refs: [String: Ident] = [:]
    var seq = 0

    func app() -> XCUIApplication { XCUIApplication(bundleIdentifier: bundle) }

    func handle(_ cmd: String, _ p: [String: Any]) throws -> [String: Any] {
        switch cmd {
        case "status":
            return ["app": bundle, "snapshot_id": snapId, "device": UIDevice.current.name, "os": UIDevice.current.systemVersion]
        case "launch":
            bundle = try str(p, "bundle_id"); app().launch(); return try settledSnapshot()
        case "activate":
            bundle = try str(p, "bundle_id"); app().activate(); return try settledSnapshot()
        case "terminate":
            let old = bundle; app().terminate(); bundle = "com.apple.springboard"; return ["terminated": old]
        case "snapshot":
            if let b = p["app"] as? String { bundle = b }
            return try settledSnapshot()
        case "tap":
            if let x = p["x"] as? Double, let y = p["y"] as? Double {
                app().coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: y)).tap()
            } else { try resolve(p["ref"]).tap() }
            return try settledSnapshot()
        case "type":
            if let r = p["ref"] { try resolve(r).tap() }
            app().typeText(try str(p, "text")); return try settledSnapshot()
        case "swipe":
            let el = p["ref"] != nil ? try resolve(p["ref"]) : app()
            switch try str(p, "direction") {
            case "up": el.swipeUp(); case "down": el.swipeDown(); case "left": el.swipeLeft(); case "right": el.swipeRight()
            default: throw DrvError(code: "BAD_REQUEST", msg: "direction up|down|left|right")
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

    func str(_ p: [String: Any], _ k: String) throws -> String {
        guard let v = p[k] as? String, !v.isEmpty else { throw DrvError(code: "BAD_REQUEST", msg: "\(k) required") }
        return v
    }

    // Per-snapshot qualified refs; re-resolve on every action and fail loudly.
    func resolve(_ any: Any?) throws -> XCUIElement {
        guard let ref = any as? String else { throw DrvError(code: "BAD_REQUEST", msg: "ref required") }
        let parts = ref.split(separator: ":")
        guard parts.count == 2, String(parts[0].dropFirst()) == snapId else {
            throw DrvError(code: "STALE_REF", msg: "current snapshot is @\(snapId); ref \(ref) is from another snapshot; re-snapshot")
        }
        guard let id = refs[ref] else { throw DrvError(code: "STALE_REF", msg: "unknown ref \(ref)") }
        let q = app().descendants(matching: id.type).matching(NSPredicate { o, _ in
            guard let a = o as? XCUIElementAttributes else { return false }
            return a.identifier == id.id && a.label == id.label && Driver.close(a.frame, id.frame)
        })
        let n = q.count
        if n == 0 { throw DrvError(code: "STALE_REF", msg: "\(ref) no longer matches a live element; re-snapshot") }
        if n > 1 { throw DrvError(code: "AMBIGUOUS_TARGET", msg: "\(ref) matches \(n) live elements; re-snapshot") }
        return q.element(boundBy: 0)
    }

    static func close(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) <= 1 && abs(a.minY - b.minY) <= 1 && abs(a.width - b.width) <= 1 && abs(a.height - b.height) <= 1
    }

    // Two-read tree-hash idle check with a finite cap (Apple's own quiescence wait is unbounded).
    func settledSnapshot(timeout: TimeInterval = 3) throws -> [String: Any] {
        let a = app()
        let deadline = Date().addingTimeInterval(timeout)
        var snap = try a.snapshot(); var reads = 1; var h = hash(snap); var settled = false
        while Date() < deadline {
            usleep(150_000)
            let s2 = try a.snapshot(); reads += 1
            let h2 = hash(s2); snap = s2
            if h2 == h { settled = true; break }
            h = h2
        }
        snapId = String(UInt64.random(in: 0...UInt64.max), radix: 36).prefix(8).description
        refs = [:]; seq = 0
        var lines: [String] = []
        let tree = build(snap, pdepth: 0, lines: &lines)
        return ["app": bundle, "snapshot_id": snapId, "ref_count": seq, "complete": true, "settled": settled,
                "reads": reads, "text": lines.joined(separator: "\n"), "tree": tree]
    }

    func hash(_ s: XCUIElementSnapshot) -> Int {
        var h = Hasher()
        func walk(_ n: XCUIElementSnapshot) {
            h.combine(n.elementType.rawValue); h.combine(Int(n.frame.minX)); h.combine(Int(n.frame.minY))
            h.combine(Int(n.frame.width)); h.combine(Int(n.frame.height)); h.combine(n.label); h.combine(n.identifier)
            h.combine(n.value.map { String(describing: $0) } ?? "")
            n.children.forEach(walk)
        }
        walk(s); return h.finalize()
    }

    func build(_ n: XCUIElementSnapshot, pdepth: Int, lines: inout [String]) -> [String: Any] {
        seq += 1
        let ref = "@\(snapId):e\(seq)"
        refs[ref] = Ident(type: n.elementType, id: n.identifier, label: n.label, frame: n.frame)
        let role = Driver.role(n.elementType)
        let name = !n.label.isEmpty ? n.label : (!n.title.isEmpty ? n.title : (n.placeholderValue ?? n.identifier))
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
            line += " at=\(Int(f.minX)),\(Int(f.minY)) size=\(Int(f.width))x\(Int(f.height))"
            if !states.isEmpty { line += " [\(states.joined(separator: ","))]" }
            lines.append(line)
        }
        var node: [String: Any] = ["role": role, "name": name, "value": value, "ref_id": ref, "states": states,
                                   "available_actions": actions,
                                   "bounds": ["x": Double(f.minX), "y": Double(f.minY), "width": Double(f.width), "height": Double(f.height)]]
        if !n.identifier.isEmpty { node["native_id"] = ["kind": "ax_identifier", "value": n.identifier] }
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
        case .textField, .secureTextField, .textView, .searchField: return ["Tap", "Type", "Clear"]
        case .`switch`, .toggle, .checkBox: return ["Tap", "Toggle"]
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
        guard rc == 0, listen(fd, 8) == 0 else { NSLog("agent-mobile: bind/listen failed errno=%d", errno); return }
        NSLog("agent-mobile: listening on %@:%d", bindAddr, Int(port))
        while true {
            let c = accept(fd, nil, nil)
            if c < 0 { continue }
            serve(c); close(c)
        }
    }

    func serve(_ c: Int32) {
        var buf = Data(); var chunk = [UInt8](repeating: 0, count: 65536)
        var headerEnd: Range<Data.Index>? = nil
        while headerEnd == nil {
            let n = read(c, &chunk, chunk.count); if n <= 0 { return }
            buf.append(chunk, count: n); headerEnd = buf.range(of: Data("\r\n\r\n".utf8))
        }
        let head = String(decoding: buf[..<headerEnd!.lowerBound], as: UTF8.self)
        var lines = head.components(separatedBy: "\r\n")
        let reqLine = lines.removeFirst().split(separator: " ")
        guard reqLine.count >= 2 else { return }
        var headers: [String: String] = [:]
        for l in lines { if let i = l.firstIndex(of: ":") { headers[l[..<i].lowercased()] = l[l.index(after: i)...].trimmingCharacters(in: .whitespaces) } }
        let len = Int(headers["content-length"] ?? "0") ?? 0
        var body = Data(buf[headerEnd!.upperBound...])
        while body.count < len { let n = read(c, &chunk, chunk.count); if n <= 0 { break }; body.append(chunk, count: n) }
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

    func testServe() {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        let port = UInt16(env["AGENT_MOBILE_PORT"] ?? "") ?? 8770
        let token = env["AGENT_MOBILE_TOKEN"] ?? ""
        let bindAddr = env["AGENT_MOBILE_BIND"] ?? "127.0.0.1"
        let srv = HTTPServer(port: port, bindAddr: bindAddr) { req in
            let cmd = String(req.path.split(separator: "?").first ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let t0 = Date()
            if token.isEmpty || req.headers["authorization"] != "Bearer \(token)" {
                return (401, ["version": "1", "ok": false, "error": ["code": "UNAUTHORIZED", "message": "Authorization: Bearer <AGENT_MOBILE_TOKEN> required"]])
            }
            if req.headers["x-agent-mobile-version"] != "1" {
                return (409, ["version": "1", "ok": false, "command": cmd, "elapsed_ms": Int(Date().timeIntervalSince(t0) * 1000), "error": ["code": "BAD_REQUEST", "message": "X-Agent-Mobile-Version must be 1"]])
            }
            let params = (try? JSONSerialization.jsonObject(with: req.body)) as? [String: Any] ?? [:]
            return DispatchQueue.main.sync {
                do {
                    let data = try AgentMobileServer.drv.handle(cmd, params)
                    return (200, ["version": "1", "ok": true, "command": cmd, "elapsed_ms": Int(Date().timeIntervalSince(t0) * 1000), "data": data])
                } catch let e as DrvError {
                    return (409, ["version": "1", "ok": false, "command": cmd, "elapsed_ms": Int(Date().timeIntervalSince(t0) * 1000), "error": ["code": e.code, "message": e.msg]])
                } catch {
                    return (500, ["version": "1", "ok": false, "command": cmd, "elapsed_ms": Int(Date().timeIntervalSince(t0) * 1000), "error": ["code": "DRIVER_ERROR", "message": "\(error)"]])
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
