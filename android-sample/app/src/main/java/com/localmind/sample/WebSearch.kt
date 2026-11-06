package com.localmind.sample

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import uniffi.lm_llm_ffi.WebSource
import uniffi.lm_llm_ffi.parseWikipediaSources

/** 検索に失敗した理由（画面に出せる文言）。 */
class WebSearchException(message: String) : Exception(message)

/**
 * 日本語版 Wikipedia の検索（API キー不要）。質問の全文ではなく、LLM が作った短い検索語だけを送る。
 * 接続先は固定（https）で、転送は追わない。応答の解釈と URL の検査は Rust 側（`parseWikipediaSources`）が行う。
 */
object WebSearch {
    internal const val HOST = "ja.wikipedia.org"
    internal const val MAX_QUERY_CHARS = 80
    private const val MAX_RESPONSE_BYTES = 512 * 1024
    private const val CONNECT_TIMEOUT_MS = 8_000
    private const val READ_TIMEOUT_MS = 10_000
    private const val TOTAL_TIMEOUT_NS = 20_000_000_000L
    private const val RESULTS = 5
    private const val EXTRACT_CHARS = 700

    @Volatile private var active: HttpURLConnection? = null

    /** 実行中の検索を止める（接続を切る）。 */
    fun cancel() {
        val conn = active ?: return
        // 画面のスレッドからでも通信の例外にならないよう、別のスレッドで切る
        Thread({ runCatching { conn.disconnect() } }, "lm-search-cancel").start()
    }

    /** 検索の URL。検索語は整えて（空白をそろえ、長さを絞って）エンコードする。 */
    internal fun buildUrl(query: String): URL {
        val q = query.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
        if (q.isEmpty()) throw WebSearchException(t("検索語が空です", "The search words are empty"))
        if (q.length > MAX_QUERY_CHARS) throw WebSearchException(t("検索語が長すぎます", "The search words are too long"))
        val enc = URLEncoder.encode(q, "UTF-8")
        return URL(
            "https://$HOST/w/api.php?action=query&generator=search&gsrsearch=$enc&gsrlimit=$RESULTS" +
                "&prop=extracts%7Cinfo&inprop=url&exintro=1&explaintext=1&exchars=$EXTRACT_CHARS&exlimit=max" +
                "&redirects=1&format=json&formatversion=2",
        )
    }

    /** 検索する。見つからなければ空のリスト。失敗は [WebSearchException]。重いので別スレッドから呼ぶこと。 */
    fun search(query: String): List<WebSource> {
        val url = buildUrl(query)
        val conn = (url.openConnection() as HttpURLConnection).apply {
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            instanceFollowRedirects = false
            setRequestProperty("User-Agent", "LocalMind/0.1 (Android; +https://github.com/taotaro1104-commits/localmind)")
            setRequestProperty("Accept", "application/json")
        }
        active = conn
        try {
            if (conn.responseCode != HttpURLConnection.HTTP_OK) throw WebSearchException(t("検索サービスが応答しませんでした", "The search service did not respond"))
            val body = conn.inputStream.use { input ->
                val out = java.io.ByteArrayOutputStream()
                val buf = ByteArray(8192)
                val deadline = System.nanoTime() + TOTAL_TIMEOUT_NS
                while (true) {
                    if (System.nanoTime() > deadline) throw WebSearchException(t("検索に時間がかかりすぎました", "The search took too long"))
                    val n = input.read(buf)
                    if (n < 0) break
                    if (out.size() + n > MAX_RESPONSE_BYTES) throw WebSearchException(t("検索の応答が大きすぎます", "The search response is too large"))
                    out.write(buf, 0, n)
                }
                out.toString(Charsets.UTF_8.name())
            }
            return parseWikipediaSources(body)
        } catch (e: WebSearchException) {
            throw e
        } catch (e: IOException) {
            throw WebSearchException(t("インターネットに接続できませんでした", "Could not connect to the internet"))
        } catch (e: RuntimeException) {
            throw WebSearchException(t("検索の応答を読めませんでした", "Could not read the search response"))
        } finally {
            active = null
            conn.disconnect()
        }
    }
}
