import XCTest
import UIKit

// The driver's entry point: one never-ending XCUITest method that hosts the
// HTTP server (WebDriverAgent shape). XCUITest is the hidden mechanism; the
// CLI only sees HTTP.
final class AgentMobileServer: XCTestCase {
    static let drv = Driver()
    static let protocolVersion = "1"

    func testServe() {
        continueAfterFailure = true
        signal(SIGPIPE, SIG_IGN)
        Runtime.killQuiescenceWait()
        let env = ProcessInfo.processInfo.environment
        let port = UInt16(env["AGENT_MOBILE_PORT"] ?? "") ?? 8770
        let token = env["AGENT_MOBILE_TOKEN"] ?? ""
        let bindAddr = env["AGENT_MOBILE_BIND"] ?? "127.0.0.1"
        let version = AgentMobileServer.protocolVersion
        let srv = HTTPServer(port: port, bindAddr: bindAddr) { req in
            let cmd = String(req.path.split(separator: "?").first ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let t0 = Date()
            let elapsed = { Int(Date().timeIntervalSince(t0) * 1000) }
            let fail = { (status: Int, code: String, msg: String) -> (Int, [String: Any]) in
                (status, ["version": version, "ok": false, "command": cmd, "elapsed_ms": elapsed(), "error": ["code": code, "message": msg]])
            }
            if token.isEmpty || req.headers["authorization"] != "Bearer \(token)" {
                return (401, ["version": version, "ok": false, "error": ["code": "UNAUTHORIZED", "message": "Authorization: Bearer <AGENT_MOBILE_TOKEN> required"]])
            }
            if req.method != "POST" { return fail(405, "BAD_REQUEST", "verbs are POST only") }
            if req.headers["x-agent-mobile-version"] != version { return fail(409, "BAD_REQUEST", "X-Agent-Mobile-Version must be 1") }
            let params = (try? JSONSerialization.jsonObject(with: req.body)) as? [String: Any] ?? [:]
            return DispatchQueue.main.sync {
                do {
                    let data = try AgentMobileServer.drv.handle(cmd, params)
                    return (200, ["version": version, "ok": true, "command": cmd, "elapsed_ms": elapsed(), "data": data])
                } catch let e as DrvError {
                    return fail(409, e.code, e.msg)
                } catch {
                    return fail(500, "DRIVER_ERROR", "\(error)")
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
