import XCTest
import UIKit

extension Driver {
    // Three-read tree-hash idle check with a finite cap (Apple's own
    // quiescence wait is unbounded). Reads are spaced by at least `gap`.
    func settledSnapshot(timeout: TimeInterval = 3, target: String? = nil) throws -> [String: Any] {
        let gap: TimeInterval = 0.15
        let a = app(target ?? bundle)
        var alert: XCUIElement?
        func read() throws -> XCUIElementSnapshot {
            let candidate = app("com.apple.springboard").alerts.firstMatch
            alert = candidate.exists && candidate.isHittable ? candidate : nil
            return try (alert ?? a).snapshot()
        }
        let started = Date(); let deadline = started.addingTimeInterval(timeout)
        var snap = try read(); var reads = 1; var h = hash(snap); var at = Date()
        var consecutiveMatches = 1
        var settled = false
        while !settled, Date() < deadline {
            let wait = gap - Date().timeIntervalSince(at)
            if wait > 0 { usleep(UInt32(wait * 1_000_000)) }
            let s2 = try read(); reads += 1
            let h2 = hash(s2); let at2 = Date(); snap = s2
            if h2 == h { consecutiveMatches += 1 } else { consecutiveMatches = 1 }
            if consecutiveMatches >= 3 { settled = true }
            h = h2; at = at2
        }
        let settleMs = Int(Date().timeIntervalSince(started) * 1000)
        lastRead = (h, at)
        snapId = newSnapId()
        refs = [:]; seq = 0
        alertTarget = alert
        var lines: [String] = []
        let tree = build(snap, pdepth: 0, lines: &lines)
        return ["app": target ?? bundle, "snapshot_id": snapId, "ref_count": seq, "complete": true, "settled": settled,
                "reads": reads, "settle_ms": settleMs, "text": lines.joined(separator: "\n"), "tree": tree]
    }

    // The settle hash covers the set of distinct nodes. Snapshots of a still
    // screen differ run to run only in sibling order or in one more or one
    // fewer copy of an identical node, and a visible change always alters
    // some node's type, label, identifier, value, or frame.
    func hash(_ s: XCUIElementSnapshot) -> Int {
        var nodes = Set<Int>()
        func walk(_ n: XCUIElementSnapshot) {
            var h = Hasher()
            h.combine(n.elementType.rawValue); h.combine(Driver.qi(n.frame.minX)); h.combine(Driver.qi(n.frame.minY))
            h.combine(Driver.qi(n.frame.width)); h.combine(Driver.qi(n.frame.height)); h.combine(Driver.snapString(n, "label")); h.combine(Driver.snapString(n, "identifier"))
            h.combine(n.value.map { String(describing: $0) } ?? "")
            nodes.insert(h.finalize())
            n.children.forEach(walk)
        }
        walk(s); return nodes.hashValue
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
}
