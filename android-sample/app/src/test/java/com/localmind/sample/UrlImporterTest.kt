package com.localmind.sample

import java.nio.charset.Charset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class UrlImporterTest {
    @Test
    fun onlyPlainHttpsHostnameUrlsAreAccepted() {
        assertEquals("example.com", UrlImporter.parse("  https://example.com/a/b?x=1#f  ").host)
        for (bad in listOf(
            "", "   ", "http://example.com/", "ftp://example.com/", "file:///etc/passwd", "javascript:alert(1)",
            "https://user:pw@example.com/", "https://user@example.com/", "https://localhost/", "https://intranet/",
            "https://127.0.0.1/", "https://10.0.0.5/x", "https://[::1]/", "https://192.168.0.1:8443/",
            "https://exa mple.com/", "https://example.com/\nx", "https://" + "a".repeat(UrlImporter.MAX_URL_CHARS) + ".com/",
        )) {
            try {
                UrlImporter.parse(bad)
                fail("拒否されるはず: $bad")
            } catch (_: IllegalArgumentException) {
            }
        }
    }

    @Test
    fun ipLiteralsAreRecognised() {
        for (ip in listOf("1.2.3.4", "[::1]", "[2001:db8::1]", "127.0.0.1")) assertTrue(ip, UrlImporter.isIpLiteral(ip))
        for (name in listOf("example.com", "a1.b2.jp", "1password.com")) assertFalse(name, UrlImporter.isIpLiteral(name))
    }

    @Test
    fun theKindFollowsTheContentType() {
        val k = UrlImporter.Kind.values()
        assertEquals(UrlImporter.Kind.HTML, UrlImporter.kindOf("text/html; charset=UTF-8", "/x"))
        assertEquals(UrlImporter.Kind.HTML, UrlImporter.kindOf("application/xhtml+xml", "/x"))
        assertEquals(UrlImporter.Kind.MARKDOWN, UrlImporter.kindOf("text/markdown", "/x"))
        assertEquals(UrlImporter.Kind.TEXT, UrlImporter.kindOf("text/plain", "/notes.txt"))
        assertEquals(UrlImporter.Kind.MARKDOWN, UrlImporter.kindOf("text/plain; charset=utf-8", "/README.MD"))
        for (other in listOf("application/pdf", "image/png", "application/octet-stream", "")) {
            assertEquals(other, UrlImporter.Kind.OTHER, UrlImporter.kindOf(other, "/x"))
        }
        assertEquals(4, k.size)
    }

    @Test
    fun theCharsetComesFromTheHeaderThenTheMetaTagAndUnknownOnesAreUtf8() {
        val sjis = Charset.forName("Shift_JIS")
        assertEquals(sjis, UrlImporter.charsetOf("text/html; charset=Shift_JIS", ByteArray(0)))
        assertEquals(Charset.forName("EUC-JP"), UrlImporter.charsetOf("text/html", """<meta charset="euc-jp">""".toByteArray()))
        assertEquals(
            sjis,
            UrlImporter.charsetOf("text/html", """<meta http-equiv="Content-Type" content="text/html; charset=shift_jis">""".toByteArray()),
        )
        assertEquals(Charsets.UTF_8, UrlImporter.charsetOf("text/html; charset=weird-9", ByteArray(0)))
        assertEquals(Charsets.UTF_8, UrlImporter.charsetOf("text/html", ByteArray(0)))
    }

    @Test
    fun decodingIsStrictUnlessTheInputWasCut() {
        val bytes = "日本語".toByteArray(Charsets.UTF_8)
        assertEquals("日本語", UrlImporter.decode(bytes, Charsets.UTF_8))
        val cut = bytes.copyOf(bytes.size - 1) // 末尾の文字が欠けた
        try {
            UrlImporter.decode(cut, Charsets.UTF_8)
            fail("欠けた入力は断る")
        } catch (_: IllegalArgumentException) {
        }
        assertTrue(UrlImporter.decode(cut, Charsets.UTF_8, strict = false).startsWith("日本"))
        assertEquals("日本語", UrlImporter.decode("日本語".toByteArray(Charset.forName("Shift_JIS")), Charset.forName("Shift_JIS")))
    }
}
