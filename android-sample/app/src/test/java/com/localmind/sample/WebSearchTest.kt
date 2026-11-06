package com.localmind.sample

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class WebSearchTest {
    @Test
    fun theUrlIsAFixedHttpsWikipediaEndpointWithTheQueryEncoded() {
        val url = WebSearch.buildUrl("東京  天気")
        assertEquals("https", url.protocol)
        assertEquals(WebSearch.HOST, url.host)
        assertEquals("/w/api.php", url.path)
        // 空白はそろえ、日本語はパーセントエンコードする
        assertTrue(url.query, url.query.contains("gsrsearch=%E6%9D%B1%E4%BA%AC+%E5%A4%A9%E6%B0%97&"))
        assertTrue(url.query.contains("gsrlimit=5"))
    }

    @Test
    fun aQueryCannotInjectParametersOrChangeTheHost() {
        val url = WebSearch.buildUrl("a&action=delete#frag/../@evil.example")
        assertEquals(WebSearch.HOST, url.host)
        assertEquals("/w/api.php", url.path)
        assertEquals("action=query", url.query.substringBefore('&'))
        assertFalse(url.query.contains("action=delete"))
        assertEquals(null, url.ref)
        assertEquals(null, url.userInfo)
    }

    @Test
    fun emptyOrOverlongQueriesAreRefused() {
        for (bad in listOf("", "   \n\t", "あ".repeat(WebSearch.MAX_QUERY_CHARS + 1))) {
            try {
                WebSearch.buildUrl(bad)
                fail("拒否されるはず: $bad")
            } catch (_: WebSearchException) {
            }
        }
        WebSearch.buildUrl("あ".repeat(WebSearch.MAX_QUERY_CHARS))
    }
}
