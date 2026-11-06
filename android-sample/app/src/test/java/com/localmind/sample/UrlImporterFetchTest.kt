package com.localmind.sample

import java.io.ByteArrayInputStream
import java.net.URL
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uniffi.lm_ffi.DocFormat
import uniffi.lm_ffi.HtmlPage

/** 通信と名前の解決を差し替えて、取得の流れ（転送・検査・上限）を確かめる。 */
class UrlImporterFetchTest : JapaneseUi() {
    private val requested = mutableListOf<String>()
    private var closed = 0

    private fun reply(code: Int, type: String? = null, body: ByteArray? = null, location: String? = null) =
        UrlImporter.Reply(code, location, type, body?.let { ByteArrayInputStream(it) }, { closed++ })

    private fun deps(
        isPublic: (String) -> Boolean = { true },
        html: (String) -> HtmlPage = { HtmlPage("題名", "# 題名\n\n本文 ${it.length}\n", false) },
        route: (URL) -> UrlImporter.Reply,
    ) = UrlImporter.Deps(
        transport = UrlImporter.Transport { requested += it.toString(); route(it) },
        isPublic = isPublic,
        html = html,
    )

    private fun fails(urlText: String, d: UrlImporter.Deps, needle: String) {
        try {
            UrlImporter.fetch(urlText, d)
            fail("失敗するはず")
        } catch (e: IllegalStateException) {
            assertTrue("${e.message}", e.message.orEmpty().contains(needle))
        }
    }

    @Test
    fun anHtmlPageBecomesATitledMarkdownDocumentWithItsSource() {
        val d = deps { reply(200, "text/html; charset=utf-8", "<p>x</p>".toByteArray()) }
        val doc = UrlImporter.fetch("https://example.com/a", d)
        assertEquals("題名", doc.title)
        assertEquals(DocFormat.MARKDOWN, doc.format)
        assertTrue(doc.text.endsWith("取得元: https://example.com/a\n"))
        assertEquals(1, closed)
    }

    @Test
    fun plainTextKeepsItsFormatAndAMarkdownFileIsMarkdown() {
        val text = deps { reply(200, "text/plain", "本文です".toByteArray()) }
        assertEquals(DocFormat.TEXT, UrlImporter.fetch("https://example.com/notes.txt", text).format)
        val md = deps { reply(200, "text/plain; charset=utf-8", "# 見出し".toByteArray()) }
        val doc = UrlImporter.fetch("https://example.com/docs/README.md", md)
        assertEquals(DocFormat.MARKDOWN, doc.format)
        assertEquals("README", doc.title)
    }

    @Test
    fun aByteOrderMarkIsDropped() {
        val d = deps { reply(200, "text/plain", byteArrayOf(0xEF.toByte(), 0xBB.toByte(), 0xBF.toByte()) + "本文".toByteArray()) }
        assertTrue(UrlImporter.fetch("https://example.com/a.txt", d).text.startsWith("本文"))
    }

    @Test
    fun redirectsAreFollowedAndEachHopIsChecked() {
        val d = deps { u ->
            when (u.path) {
                "/a" -> reply(302, location = "/b")
                "/b" -> reply(301, location = "https://other.example.org/c")
                else -> reply(200, "text/plain", "ok".toByteArray())
            }
        }
        val doc = UrlImporter.fetch("https://example.com/a", d)
        assertEquals(listOf("https://example.com/a", "https://example.com/b", "https://other.example.org/c"), requested)
        assertTrue(doc.text.endsWith("取得元: https://other.example.org/c\n"))
        assertEquals(3, closed)
    }

    @Test
    fun aRedirectToANonPublicHostHttpAnOddPortOrAnIpIsRefusedWithoutConnecting() {
        for (target in listOf(
            "https://intranet.example.net/x", "http://example.com/x", "https://example.com:8443/x", "https://127.0.0.1/x",
            "https://user:pw@example.com/x", "ftp://example.com/x",
        )) {
            requested.clear()
            val d = deps(isPublic = { it != "intranet.example.net" }) { reply(302, location = target) }
            fails("https://example.com/a", d, "") // 文言は理由ごとに違う（接続しないことが肝心）
            assertEquals("転送先には接続しない: $target", listOf("https://example.com/a"), requested)
        }
    }

    @Test
    fun aNonPublicFirstHostIsRefusedWithoutConnecting() {
        val d = deps(isPublic = { false }) { reply(200, "text/plain", "x".toByteArray()) }
        fails("https://example.com/a", d, "公開されていない")
        assertEquals(emptyList<String>(), requested)
    }

    @Test
    fun tooManyRedirectsAndMissingLocationsFail() {
        fails("https://example.com/a", deps { reply(302, location = "/loop") }, "転送が多すぎます")
        assertEquals(UrlImporter.MAX_REDIRECTS + 1, requested.size)
        fails("https://example.com/a", deps { reply(302) }, "転送先を読めません")
    }

    @Test
    fun errorStatusesAndForbiddenTypesFailAndAlwaysClose() {
        fails("https://example.com/a", deps { reply(404, "text/html") }, "404")
        assertEquals(1, closed)
        fails("https://example.com/a", deps { reply(200, "application/pdf", ByteArray(10)) }, "この種類")
        fails("https://example.com/a", deps { reply(200, null, ByteArray(10)) }, "この種類")
        assertEquals(3, closed)
    }

    @Test
    fun oversizeTextIsRefusedButOversizeHtmlIsCutAndStillRead() {
        val big = ByteArray(Importer.MAX_BYTES) { 'a'.code.toByte() }
        fails("https://example.com/a.txt", deps { reply(200, "text/plain", big) }, "大きすぎ")
        var seen = 0
        val d = deps(html = { seen = it.length; HtmlPage("t", "本文\n", true) }) {
            reply(200, "text/html", ByteArray(UrlImporter.MAX_HTML_BYTES + 5000) { 'a'.code.toByte() })
        }
        val doc = UrlImporter.fetch("https://example.com/a", d)
        assertEquals(UrlImporter.MAX_HTML_BYTES, seen)
        assertTrue(doc.text.contains("後ろは取り込んでいません"))
    }

    @Test
    fun anEmptyExtractionAndABadCharsetFail() {
        val empty = deps(html = { HtmlPage("", "  \n", false) }) { reply(200, "text/html", "<p></p>".toByteArray()) }
        fails("https://example.com/a", empty, "本文を取り出せません")
        try {
            UrlImporter.fetch("https://example.com/a.txt", deps { reply(200, "text/plain", byteArrayOf(0xFF.toByte(), 0xFE.toByte(), 0x41)) })
            fail("不正な UTF-8 は断る")
        } catch (_: IllegalArgumentException) {
        }
    }

    @Test
    fun cancellingBetweenHopsStopsTheFetch() {
        val d = deps { u ->
            UrlImporter.cancel()
            reply(302, location = "/next")
        }
        fails("https://example.com/a", d, "中止")
        assertEquals(1, requested.size)
    }

    @Test
    fun theTitleFallsBackToTheHostAndIsCleaned() {
        val d = deps(html = { HtmlPage("", "本文\n", false) }) { reply(200, "text/html", "x".toByteArray()) }
        assertEquals("example.com", UrlImporter.fetch("https://example.com/a", d).title)
    }
}
