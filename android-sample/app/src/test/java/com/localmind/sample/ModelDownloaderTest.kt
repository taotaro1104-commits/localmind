package com.localmind.sample

import java.net.URL
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class ModelDownloaderTest : JapaneseUi() {
    private val mb = 1024L * 1024

    @Test
    fun trustedHostsAreTheModelHostAndItsDeliveryDomainsOnly() {
        assertTrue(ModelDownloader.isAllowedHost("huggingface.co"))
        assertTrue(ModelDownloader.isAllowedHost("us.aws.cdn.hf.co"))
        assertTrue(ModelDownloader.isAllowedHost("HF.CO".replace("HF.CO", "CDN.HF.CO")))
        for (bad in listOf("hf.co", "evilhf.co", "cdn.hf.co.evil.com", "huggingface.co.evil.com", "nothuggingface.co", "", null)) {
            assertFalse("$bad", ModelDownloader.isAllowedHost(bad))
        }
    }

    @Test
    fun redirectsNeedHttpsAndNoCredentials() {
        assertTrue(ModelDownloader.isSafeRedirect(URL("https://cdn.hf.co/x?y=1")))
        assertFalse(ModelDownloader.isSafeRedirect(URL("http://cdn.hf.co/x")))
        assertFalse(ModelDownloader.isSafeRedirect(URL("https://user:pw@cdn.hf.co/x")))
        assertFalse(ModelDownloader.isSafeRedirect(URL("https://cdn.hf.co@evil.com/x")))
        assertFalse(ModelDownloader.isSafeRedirect(URL("https://evil.com/cdn.hf.co")))
        // 直接入れた URL は、ホストを限らない（https と認証情報なしは同じ）
        assertTrue(ModelDownloader.isSafeRedirect(URL("https://example.com/m.gguf"), trustedOnly = false))
        assertFalse(ModelDownloader.isSafeRedirect(URL("http://example.com/m.gguf"), trustedOnly = false))
    }

    @Test
    fun customUrlsAreCheckedAndErrorsNameTheirField() {
        ModelSource.custom("https://example.com/m.gguf", null)
        ModelSource.custom("https://example.com/m.gguf", "A".repeat(64)) // 大文字は小文字にして受け入れる
        for (bad in listOf("http://example.com/m", "ftp://example.com/m", "file:///data/m", "https://u:p@example.com/m", "not a url", "")) {
            assertEquals("url", assertThrows(ModelDownloadException::class.java) { ModelSource.custom(bad, null) }.field)
        }
        assertEquals("sha", assertThrows(ModelDownloadException::class.java) { ModelSource.custom("https://example.com/m", "xyz") }.field)
    }

    @Test
    fun localAddressesAreRefusedAsRedirectTargets() {
        for (local in listOf("127.0.0.1", "10.0.0.5", "192.168.1.1", "172.16.0.9", "169.254.1.1", "[::1]", "[fd00::1]", "0.0.0.0")) {
            assertTrue(local, ModelDownloader.isLocalAddress(local))
        }
        assertFalse(ModelDownloader.isLocalAddress("8.8.8.8"))
    }

    @Test
    fun totalSizeComesFromContentRangeForResumesAndContentLengthOtherwise() {
        val size = 100 * mb
        assertEquals(size, ModelDownloader.parseTotal(200, null, size, 0, null))
        assertEquals(size, ModelDownloader.parseTotal(206, "bytes 5242880-104857599/$size", -1, 5 * mb, null))
        // 要求した開始位置と違う範囲は断る
        assertEquals(-1, ModelDownloader.parseTotal(206, "bytes 0-99/$size", -1, 5 * mb, null))
        assertEquals(-1, ModelDownloader.parseTotal(206, null, -1, 5 * mb, null))
        assertEquals(-1, ModelDownloader.parseTotal(206, "bytes 5242880-9/abc", -1, 5 * mb, null))
    }

    @Test
    fun sizesOutsideTheLimitsOrDifferentFromThePinnedSizeAreRefused() {
        assertEquals(-1, ModelDownloader.parseTotal(200, null, -1, 0, null)) // サイズ不明
        assertEquals(-1, ModelDownloader.parseTotal(200, null, mb - 1, 0, null)) // 小さすぎる
        assertEquals(-1, ModelDownloader.parseTotal(200, null, 8L * 1024 * mb + 1, 0, null)) // 8GiB 超
        assertEquals(50 * mb, ModelDownloader.parseTotal(200, null, 50 * mb, 0, 50 * mb))
        assertEquals(-1, ModelDownloader.parseTotal(200, null, 51 * mb, 0, 50 * mb)) // 固定したサイズと違う
        assertNull(null)
    }
}
