package com.lahfir.agentmobile.driver

import java.io.IOException
import java.io.InputStream

internal data class HttpTestResponse(val status: Int, val headers: Map<String, String>, val body: String)

/// Bounded HTTP/1.x reply reader for tests: bytewise head to CRLFCRLF
/// (64KiB cap), exactly one ASCII-digit Content-Length <=64MiB, then
/// exactly the declared body bytes decoded as UTF-8. Returns null ONLY
/// when the stream ends before a single byte (deliberate no-response);
/// any partial/truncated/malformed reply throws IOException.
internal fun readHttpResponseForTest(input: InputStream): HttpTestResponse? {
    val headCap = 64 * 1024
    val head = java.io.ByteArrayOutputStream()
    var window = 0
    var readAny = false
    while (true) {
        val b = input.read()
        if (b < 0) {
            if (!readAny) return null
            throw IOException("EOF inside response head")
        }
        readAny = true
        head.write(b)
        window = (window shl 8) or b
        if (window == 0x0D0A0D0A) break
        if (head.size() >= headCap) {
            throw IOException("response head exceeds cap")
        }
    }
    val headText = String(head.toByteArray(), Charsets.ISO_8859_1)
    val lines = headText.removeSuffix("\r\n\r\n").split("\r\n")
    val parts = lines[0].split(" ")
    if (parts.size < 3 || !parts[0].startsWith("HTTP/1.")) {
        throw IOException("malformed status line")
    }
    val status = parts[1].toIntOrNull() ?: throw IOException("malformed status")
    val headers = LinkedHashMap<String, String>()
    var contentLength = -1
    var contentLengthSeen = false
    for (i in 1 until lines.size) {
        val line = lines[i]
        if (line.isEmpty()) continue
        val sep = line.indexOf(':')
        if (sep < 0) throw IOException("malformed header line")
        val name = line.substring(0, sep).trim().lowercase()
        if (name.isEmpty()) throw IOException("empty header name")
        val value = line.substring(sep + 1).trim()
        if (name == "content-length") {
            if (contentLengthSeen) throw IOException("duplicate content-length")
            contentLengthSeen = true
            if (value.isEmpty() || value.any { it !in '0'..'9' }) {
                throw IOException("invalid content-length")
            }
            val parsed = value.toLongOrNull() ?: throw IOException("invalid content-length")
            if (parsed > 64 * 1024 * 1024) throw IOException("content-length too large")
            contentLength = parsed.toInt()
        }
        headers.putIfAbsent(name, value)
    }
    if (contentLength < 0) throw IOException("missing content-length")
    val body = ByteArray(contentLength)
    var off = 0
    while (off < contentLength) {
        val n = input.read(body, off, contentLength - off)
        if (n <= 0) throw IOException("truncated response body")
        off += n
    }
    return HttpTestResponse(status, headers, String(body, Charsets.UTF_8))
}
