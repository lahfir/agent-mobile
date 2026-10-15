package com.lahfir.agentmobile.driver

import java.io.ByteArrayInputStream
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class HttpTestResponseTest {

    private fun reply(raw: String): HttpTestResponse? =
        readHttpResponseForTest(ByteArrayInputStream(raw.toByteArray(Charsets.ISO_8859_1)))

    @Test
    fun emptyInputReturnsNull() {
        assertNull(reply(""))
    }

    @Test
    fun partialHeadWithoutTerminatorThrows() {
        try {
            reply("HTTP/1.1 200 OK\r\nContent-Length: 4")
            org.junit.Assert.fail("must throw")
        } catch (e: IOException) {
        }
    }

    @Test
    fun missingDuplicateInvalidContentLengthThrows() {
        for (raw in listOf(
            "HTTP/1.1 200 OK\r\n\r\nbody",
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            "HTTP/1.1 200 OK\r\nContent-Length: xx\r\n\r\n{}",
        )) {
            try {
                reply(raw)
                org.junit.Assert.fail("must throw: ${raw.substring(0, 20)}")
            } catch (e: IOException) {
            }
        }
    }

    @Test
    fun truncatedBodyThrows() {
        try {
            reply("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}")
            org.junit.Assert.fail("must throw")
        } catch (e: IOException) {
        }
    }

    @Test
    fun incompleteHeadAt64KiBCapThrows() {
        val raw = "HTTP/1.1 200 OK\r\n" + "X-Pad: " + "a".repeat(64 * 1024)
        try {
            reply(raw)
            org.junit.Assert.fail("must throw")
        } catch (e: IOException) {
        }
    }

    @Test
    fun utf8BodyDecodesExactly() {
        val body = "\u00e9"
        val bytes = body.toByteArray(Charsets.UTF_8)
        assertEquals(2, bytes.size)
        val head = "HTTP/1.1 200 OK\r\nContent-Length: ${bytes.size}\r\n\r\n"
        val raw = head.toByteArray(Charsets.ISO_8859_1) + bytes
        val resp = readHttpResponseForTest(ByteArrayInputStream(raw))
        assertNotNull(resp)
        assertEquals("\u00e9", resp!!.body)
    }

    @Test
    fun valid200And503Parse() {
        val ok = reply("HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}")
        assertNotNull(ok)
        assertEquals(200, ok!!.status)
        assertEquals("{\"ok\":true}", ok.body)
        val busy = reply("HTTP/1.1 503 Error\r\nContent-Length: 12\r\n\r\n{\"ok\":false}")
        assertEquals(503, busy!!.status)
        assertEquals("{\"ok\":false}", busy.body)
    }
}
