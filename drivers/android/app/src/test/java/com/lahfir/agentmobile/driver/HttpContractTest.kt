package com.lahfir.agentmobile.driver

import java.net.Socket
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/// Shared android-http-contract.json conformance: identical request
/// bytes and expectations to the Rust spec_tests consumer.
class HttpContractTest {

    private fun resource(name: String): String =
        requireNotNull(javaClass.getResourceAsStream("/$name")) {
            "missing test resource $name"
        }.bufferedReader().readText()

    private val spec by lazy { JSONObject(resource("android-http-contract.json")) }

    private fun jsonEqual(a: Any?, b: Any?): Boolean {
        if (a is JSONObject && b is JSONObject) {
            if (a.length() != b.length()) return false
            val keys = a.keys()
            while (keys.hasNext()) {
                val k = keys.next()
                if (!b.has(k) || !jsonEqual(a.get(k), b.get(k))) return false
            }
            return true
        }
        if (a is JSONArray && b is JSONArray) {
            if (a.length() != b.length()) return false
            return (0 until a.length()).all { jsonEqual(a.get(it), b.get(it)) }
        }
        if (a is Number && b is Number) {
            return a.toLong() == b.toLong() && a.toDouble() == b.toDouble()
        }
        return a == b
    }

    private fun rawRequest(case: JSONObject): ByteArray {
        val requestLine = if (case.has("request_line")) {
            case.getString("request_line")
        } else {
            spec.getString("request_line")
        }
        var head = "$requestLine\r\n${spec.getString("base_headers")}${case.getString("framing_headers")}\r\n"
        val target = case.optLong("head_bytes", -1)
        if (target >= 0) {
            val unpadded = head.replace("{pad}", "")
            val padLen = (target - unpadded.toByteArray(Charsets.ISO_8859_1).size).toInt()
            head = unpadded.replaceFirst("X-Pad: ", "X-Pad: " + "a".repeat(padLen))
            assertEquals(target.toInt(), head.toByteArray(Charsets.ISO_8859_1).size)
        }
        val bodyBytes = case.getString("body").toByteArray(Charsets.UTF_8)
        return head.toByteArray(Charsets.ISO_8859_1) + bodyBytes
    }

    private fun exchange(port: Int, raw: ByteArray): HttpTestResponse {
        Socket("127.0.0.1", port).use { socket ->
            socket.soTimeout = 5_000
            socket.getOutputStream().apply {
                write(raw)
                flush()
            }
            socket.shutdownOutput()
            val resp = readHttpResponseForTest(socket.getInputStream())
            assertNotNull("reply must be complete", resp)
            return resp!!
        }
    }

    @Test
    fun contractCasesDriveRealServer() {
        val cases = spec.getJSONArray("cases")
        assertTrue(cases.length() > 0)
        for (i in 0 until cases.length()) {
            val case = cases.getJSONObject(i)
            val name = case.getString("name")
            val calls = AtomicInteger(0)
            var captured: JSONObject? = null
            val server = HttpServer(
                token = spec.getString("token"),
                handler = { _, params, _ ->
                    calls.incrementAndGet()
                    captured = params
                    JSONObject().put("app", "x")
                },
                clockMs = { System.nanoTime() / 1_000_000 },
            )
            server.use { srv ->
                srv.start()
                val resp = exchange(srv.localPort, rawRequest(case))
                assertEquals("$name status", case.getInt("status"), resp.status)
                assertEquals("$name handler_calls", case.getInt("handler_calls"), calls.get())
                val body = JSONObject(resp.body)
                assertEquals("$name version", Protocol.VERSION, body.getString("version"))
                if (case.has("error_code")) {
                    assertFalse("$name ok", body.getBoolean("ok"))
                    assertEquals(
                        "$name error code",
                        case.getString("error_code"),
                        body.getJSONObject("error").getString("code"),
                    )
                }
                if (case.getInt("handler_calls") == 1) {
                    assertTrue("$name ok", body.getBoolean("ok"))
                    assertNotNull("$name params", captured)
                    val expected = if (case.has("expected_body")) {
                        case.getString("expected_body")
                    } else {
                        case.optString("body")
                    }
                    val expectedJson = if (expected.isEmpty()) JSONObject() else JSONObject(expected)
                    assertTrue(
                        "$name params json",
                        jsonEqual(expectedJson, captured),
                    )
                }
            }
        }
    }

    @Test
    fun metadataAndErrorFixturesMatchProduction() {
        assertEquals(Protocol.VERSION, spec.getString("version"))
        assertEquals(HttpServer.HEAD_CAP, spec.getInt("head_cap"))
        assertEquals(HttpServer.BODY_CAP, spec.getInt("body_cap"))
        val cases = spec.getJSONArray("cases")
        val names = (0 until cases.length()).map { cases.getJSONObject(it).getString("name") }
        assertTrue(names.any { it.contains("chunked") })
        assertTrue(names.any { it.contains("duplicate_content_length") })
        val fixtures = spec.getJSONArray("error_fixtures")
        assertEquals(6, fixtures.length())
        for (i in 0 until fixtures.length()) {
            val ref = fixtures.getJSONObject(i)
            val captured = JSONObject(resource(ref.getString("file")))
            val error = captured.getJSONObject("error")
            val command = captured.optString("command").ifEmpty { null }
            val elapsed = if (captured.has("elapsed_ms")) captured.getLong("elapsed_ms") else null
            val rebuilt = Protocol.failure(
                command,
                elapsed,
                error.getString("code"),
                error.getString("message"),
            )
            assertTrue("${ref.getString("file")} envelope", jsonEqual(rebuilt, captured))
            assertEquals(
                "${ref.getString("file")} status",
                ref.getInt("status"),
                Protocol.statusForCode(error.getString("code")),
            )
        }
    }
}
