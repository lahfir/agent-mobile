package com.lahfir.agentmobile.driver

import java.io.IOException

internal class HttpFramingException(message: String) : IOException(message)

/// Parsed request head: request line plus first-value headers. Mirrors
/// the Rust parse_head contract: exactly `METHOD PATH HTTP/1.0|1.1`,
/// no Transfer-Encoding, a single ASCII-digit Content-Length bounded by
/// maxBodyBytes, other duplicate fields keep the first value.
internal data class HttpRequestHead(
    val method: String,
    val path: String,
    val headers: Map<String, String>,
    val contentLength: Int,
) {
    companion object {
        fun parse(headBytes: ByteArray, maxBodyBytes: Int): HttpRequestHead {
            val head = String(headBytes, Charsets.ISO_8859_1)
            val lines = head.split("\r\n")
            val words = lines[0].trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
            if (words.size != 3) {
                throw HttpFramingException("bad request line")
            }
            val (method, path, version) = Triple(words[0], words[1], words[2])
            if (version != "HTTP/1.0" && version != "HTTP/1.1") {
                throw HttpFramingException("bad http version")
            }
            val headers = LinkedHashMap<String, String>()
            var contentLength = 0
            var contentLengthSeen = false
            for (i in 1 until lines.size) {
                val line = lines[i]
                if (line.isEmpty()) continue
                val sep = line.indexOf(':')
                if (sep < 0) {
                    throw HttpFramingException("bad header line")
                }
                val name = line.substring(0, sep).trim().lowercase()
                if (name.isEmpty()) {
                    throw HttpFramingException("bad header name")
                }
                val value = line.substring(sep + 1).trim()
                if (name == "transfer-encoding") {
                    throw HttpFramingException("transfer-encoding unsupported")
                }
                if (name == "content-length") {
                    if (contentLengthSeen) {
                        throw HttpFramingException("duplicate content-length")
                    }
                    contentLengthSeen = true
                    if (value.isEmpty() || value.any { it !in '0'..'9' }) {
                        throw HttpFramingException("bad content-length")
                    }
                    val parsed = value.toLongOrNull()
                        ?: throw HttpFramingException("bad content-length")
                    if (parsed > maxBodyBytes) {
                        throw HttpFramingException("body too large")
                    }
                    contentLength = parsed.toInt()
                }
                headers.putIfAbsent(name, value)
            }
            return HttpRequestHead(method, path, headers, contentLength)
        }
    }
}
