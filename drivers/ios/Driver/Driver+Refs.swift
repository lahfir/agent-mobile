import XCTest
import UIKit

struct Ident { let type: XCUIElement.ElementType; let id: String; let label: String; let frame: CGRect }

extension Driver {
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

    // Per-snapshot qualified refs; re-resolve on every action and fail loudly.
    // One fresh tree read is walked in-process for (type, identifier, label,
    // frame ±1 pt) instead of a predicate query that XCTest evaluates twice;
    // the caller acts on the matched frame's coordinates. The read's hash and
    // timestamp are returned so the settle check can count it as the first
    // consecutive read only when it is old enough to prove an interval.
    func resolve(_ any: Any?) throws -> (node: XCUIElementSnapshot, read: (h: Int, at: Date)) {
        let (ref, id) = try lookupIdent(any)
        let snap = try app().snapshot()
        let at = Date()
        var hits: [XCUIElementSnapshot] = []
        func walk(_ node: XCUIElementSnapshot) {
            if node.elementType == id.type, Driver.snapString(node, "identifier") == id.id, Driver.snapString(node, "label") == id.label,
               Driver.close(node.frame, id.frame) { hits.append(node) }
            node.children.forEach(walk)
        }
        walk(snap)
        if hits.isEmpty { throw DrvError(code: "STALE_REF", msg: "\(ref) no longer matches a live element; re-snapshot") }
        if hits.count > 1 { throw DrvError(code: "AMBIGUOUS_TARGET", msg: "\(ref) matches \(hits.count) live elements; re-snapshot") }
        return (hits[0], (hash(snap), at))
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
}
