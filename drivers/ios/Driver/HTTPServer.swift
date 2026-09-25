import Foundation

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
