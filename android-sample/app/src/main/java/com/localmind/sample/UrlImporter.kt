package com.localmind.sample

import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.Charset
import java.nio.charset.CodingErrorAction
import uniffi.lm_ffi.DocFormat
import uniffi.lm_ffi.htmlToMarkdown

/**
 * URL の資料を取得して、取り込める形にする。利用者が入れた URL へ GET を 1 回送るだけ（Cookie・認証情報は送らない）。
 * 入力は信用しない:
 * - https のみ。認証情報つき・IP アドレスの直書き・自端末や LAN のアドレスへ向かう接続は断る。転送は 5 回まで、
 *   転送先も同じ検査を通す。
 * - 受け付ける種類は HTML・テキスト・Markdown だけ。読む量（展開後）と時間に上限がある。
 * - HTML は Rust 側（`htmlToMarkdown`）で本文だけにする。
 * 名前の解決結果は接続まで固定できない（標準の接続が名前から再解決する）ので、接続した相手のアドレスも、TLS を始める前に
 * [NetGuard.Guarded] で検査する（リバインディング対策）。プロキシ経由だと相手がプロキシになるので、プロキシが私設アドレス
 * のときは、取得は失敗する（安全側に倒れる）。
 */
object UrlImporter {
    internal const val MAX_REDIRECTS = 5
    internal const val MAX_HTML_BYTES = 2 * 1024 * 1024
    private const val CONNECT_TIMEOUT_MS = 10_000
    private const val READ_TIMEOUT_MS = 15_000
    private const val TOTAL_TIMEOUT_NS = 30_000_000_000L
    internal const val MAX_URL_CHARS = 2000
    /** 本文のあとに足す「取得元」の行のために、残しておく余地。 */
    private const val SOURCE_NOTE_BYTES = 4096
    private val CHARSETS = mapOf(
        "utf-8" to "UTF-8", "utf8" to "UTF-8",
        "shift_jis" to "Shift_JIS", "shift-jis" to "Shift_JIS", "sjis" to "Shift_JIS", "x-sjis" to "Shift_JIS",
        "windows-31j" to "Windows-31J", "ms932" to "Windows-31J",
        "euc-jp" to "EUC-JP", "euc_jp" to "EUC-JP",
    )

    @Volatile private var active: HttpURLConnection? = null
    @Volatile private var cancelled = false

    /** 実行中の取得を止める。 */
    fun cancel() {
        cancelled = true
        val conn = active ?: return
        Thread({ runCatching { conn.disconnect() } }, "lm-url-cancel").start()
    }

    /** 入力の URL を検査する（名前の解決はしない）。問題があれば、利用者に見せられる文言の例外。 */
    internal fun parse(text: String): URL {
        val raw = text.trim()
        require(raw.isNotEmpty()) { t("URL を入力してください", "Please enter a URL") }
        require(raw.length <= MAX_URL_CHARS) { t("URL が長すぎます", "The URL is too long") }
        require(raw.none { it.isISOControl() || it.isWhitespace() }) { t("URL に空白や制御文字は使えません", "The URL cannot contain spaces or control characters") }
        val url = try {
            URL(raw)
        } catch (_: Exception) {
            throw IllegalArgumentException(t("URL の形が正しくありません", "The URL is not in a valid form"))
        }
        require(url.protocol == "https") { t("https:// で始まる URL だけ取り込めます", "Only URLs that start with https:// can be imported") }
        require(url.userInfo == null) { t("ユーザー名やパスワードを含む URL は取り込めません", "URLs that contain a user name or password cannot be imported") }
        require(url.port == -1 || url.port == 443) { t("ポート番号を指定した URL は取り込めません（443 だけ）", "URLs with a port number cannot be imported (443 only)") }
        val host = url.host.orEmpty()
        require(host.isNotEmpty() && host.contains('.')) { t("URL のホスト名が正しくありません（社内の短い名前や localhost は使えません）", "The host name in the URL is not valid (short internal names and localhost cannot be used)") }
        require(!isIpLiteral(host)) { t("IP アドレスの直接指定は取り込めません。ホスト名の URL を入れてください", "IP addresses cannot be used directly. Enter a URL with a host name") }
        return url
    }

    /** IPv4・IPv6 の直書き（`[::1]` を含む）か。 */
    internal fun isIpLiteral(host: String): Boolean =
        host.startsWith("[") || host.all { it.isDigit() || it == '.' }

    /** 応答（読み終えたら [close] する）。 */
    internal class Reply(val code: Int, val location: String?, val contentType: String?, val body: java.io.InputStream?, val close: () -> Unit)

    /** 1 回の GET。テストでは、通信せずに応答を返すものに差し替える。 */
    internal fun interface Transport {
        fun get(url: URL): Reply
    }

    /** 外の世界との接点（通信・名前の解決・HTML の変換）。テストで差し替える。 */
    internal class Deps(
        val transport: Transport = Transport { UrlImporter.realGet(it) },
        val isPublic: (String) -> Boolean = NetGuard::resolvesOnlyToPublic,
        val html: (String) -> uniffi.lm_ffi.HtmlPage = ::htmlToMarkdown,
    )

    /** 取得して、資料にする。重いので別スレッドから呼ぶこと。失敗は、利用者に見せられる文言の例外。 */
    fun fetch(urlText: String): Imported = fetch(urlText, Deps())

    internal fun fetch(urlText: String, deps: Deps): Imported {
        val first = parse(urlText)
        cancelled = false
        val deadline = System.nanoTime() + TOTAL_TIMEOUT_NS
        try {
            val (reply, finalUrl) = open(first, deadline, deps)
            try {
                return read(reply, finalUrl, deadline, deps)
            } finally {
                reply.close()
            }
        } catch (e: javax.net.ssl.SSLException) {
            throw IllegalStateException(t("安全な接続を確立できませんでした", "Could not make a secure connection"))
        } catch (e: IOException) {
            // 中止のために接続を切ったときの例外は、失敗ではなく中止として伝える
            throw IllegalStateException(if (cancelled) t("中止しました", "Stopped") else t("ページを取得できませんでした（接続を確認してください）", "Could not fetch the page (please check your connection)"))
        }
    }

    /** 実際の通信。接続した相手のアドレスも、TLS を始める前に検査する（名前の解決と接続のあいだの差し替えを防ぐ）。 */
    internal fun realGet(url: URL): Reply {
        val c = url.openConnection() as HttpURLConnection
        (c as? javax.net.ssl.HttpsURLConnection)?.sslSocketFactory =
            NetGuard.Guarded(javax.net.ssl.HttpsURLConnection.getDefaultSSLSocketFactory())
        c.instanceFollowRedirects = false
        c.connectTimeout = CONNECT_TIMEOUT_MS
        c.readTimeout = READ_TIMEOUT_MS
        c.useCaches = false
        c.setRequestProperty("User-Agent", "LocalMind/0.1 (Android; +https://github.com/taotaro1104-commits/localmind)")
        c.setRequestProperty("Accept", "text/html,text/plain,text/markdown;q=0.9")
        active = c
        val close = { active = null; c.disconnect() }
        if (cancelled) { // 接続を登録する前に届いた中止も有効にする
            close()
            throw IllegalStateException(t("中止しました", "Stopped"))
        }
        return try {
            val code = c.responseCode
            Reply(code, c.getHeaderField("Location"), c.contentType, if (code == HttpURLConnection.HTTP_OK) c.inputStream else null, close)
        } catch (e: IOException) {
            close()
            throw e
        }
    }

    /** 転送を、1 回ずつ検査しながら辿る。 */
    private fun open(start: URL, deadline: Long, deps: Deps): Pair<Reply, URL> {
        var url = start
        repeat(MAX_REDIRECTS + 1) {
            check(url.protocol == "https" && url.userInfo == null && !isIpLiteral(url.host)) { t("想定外の接続先のため中止しました", "Stopped because the destination was not expected") }
            check(url.port == -1 || url.port == 443) { t("想定外の接続先のため中止しました", "Stopped because the destination was not expected") }
            check(System.nanoTime() < deadline) { t("ページの取得に時間がかかりすぎました", "Fetching the page took too long") }
            check(!cancelled) { t("中止しました", "Stopped") }
            check(deps.isPublic(url.host)) { t("このアドレスには接続できません（公開されていないアドレスです）", "Cannot connect to this address (it is not a public address)") }
            val reply = deps.transport.get(url)
            val code = reply.code
            if (code in 300..399) {
                val next = reply.location
                reply.close()
                check(!next.isNullOrBlank()) { t("転送先を読めません", "Cannot read the redirect target") }
                url = try { URL(url, next) } catch (_: Exception) { throw IllegalStateException(t("転送先を読めません", "Cannot read the redirect target")) }
            } else {
                if (code != HttpURLConnection.HTTP_OK || reply.body == null) {
                    reply.close()
                    error(t("ページを取得できません（サーバーの応答 $code）", "Cannot fetch the page (the server answered $code)"))
                }
                return reply to url
            }
        }
        throw IllegalStateException(t("転送が多すぎます", "Too many redirects"))
    }

    private fun read(reply: Reply, url: URL, deadline: Long, deps: Deps): Imported {
        val type = reply.contentType.orEmpty()
        val kind = kindOf(type, url.path)
        check(kind != Kind.OTHER) { t("この種類のページは取り込めません（HTML・テキスト・Markdown だけ）", "This kind of page cannot be imported (only HTML, text and Markdown)") }
        val limit = if (kind == Kind.HTML) MAX_HTML_BYTES else Importer.MAX_BYTES - SOURCE_NOTE_BYTES
        var cut = false
        val bytes = checkNotNull(reply.body).use { input ->
            val out = ByteArrayOutputStream()
            val buf = ByteArray(8192)
            while (true) {
                check(System.nanoTime() < deadline) { t("ページの取得に時間がかかりすぎました", "Fetching the page took too long") }
                check(!cancelled) { t("中止しました", "Stopped") }
                val n = input.read(buf)
                if (n < 0) break
                if (out.size() + n > limit) {
                    // HTML は上限までで切る（本文は Rust 側で絞る）。テキストは、切ると意味が変わるので断る。
                    check(kind == Kind.HTML) { t("ページが大きすぎます（${Importer.MAX_BYTES / 1024}KB まで）", "The page is too large (up to ${Importer.MAX_BYTES / 1024} KB)") }
                    out.write(buf, 0, limit - out.size())
                    cut = true
                    break
                }
                out.write(buf, 0, n)
            }
            out.toByteArray()
        }
        // 切ったときは、末尾の文字が欠けてもよい。先頭の BOM は除く。
        val text = decode(bytes, charsetOf(type, bytes), strict = !cut).removePrefix("\uFEFF")
        val source = t("取得元: ${url.toString().take(300)}", "Source: ${url.toString().take(300)}")
        return when (kind) {
            Kind.HTML -> {
                val page = deps.html(text)
                check(page.markdown.isNotBlank()) { t("ページから本文を取り出せませんでした", "Could not extract the main text from the page") }
                val title = page.title.ifBlank { url.host }
                val note = if (page.truncated) t("（本文が大きいため、後ろは取り込んでいません）\n", "(The text is large, so the rest was not imported)\n") else ""
                Imported(Importer.cleanTitle(title), page.markdown + "\n" + note + source + "\n", DocFormat.MARKDOWN)
            }
            Kind.MARKDOWN, Kind.TEXT -> {
                val fmt = if (kind == Kind.MARKDOWN) DocFormat.MARKDOWN else DocFormat.TEXT
                val last = url.path.substringAfterLast('/').substringBeforeLast('.')
                Imported(Importer.cleanTitle(last.ifBlank { url.host }), text.trimEnd() + "\n\n" + source + "\n", fmt)
            }
            Kind.OTHER -> error("unreachable")
        }
    }

    internal enum class Kind { HTML, MARKDOWN, TEXT, OTHER }

    /** 応答の種類（Content-Type）から、扱い方を決める。`text/plain` の `.md` は Markdown として扱う。 */
    internal fun kindOf(contentType: String, path: String): Kind {
        val mime = contentType.substringBefore(';').trim().lowercase()
        val md = path.lowercase().let { p -> listOf(".md", ".markdown", ".mkd").any { p.endsWith(it) } }
        return when (mime) {
            "text/html", "application/xhtml+xml" -> Kind.HTML
            "text/markdown", "text/x-markdown" -> Kind.MARKDOWN
            "text/plain" -> if (md) Kind.MARKDOWN else Kind.TEXT
            else -> Kind.OTHER
        }
    }

    /** 文字コード。応答の `charset=`、無ければ HTML の先頭の `<meta charset=…>`。知らないものは UTF-8 とみなす。 */
    internal fun charsetOf(contentType: String, bytes: ByteArray): Charset {
        val fromHeader = Regex("charset=\"?([A-Za-z0-9_-]+)", RegexOption.IGNORE_CASE).find(contentType)?.groupValues?.get(1)
        val head = String(bytes, 0, minOf(bytes.size, 2048), Charsets.ISO_8859_1)
        val fromMeta = Regex("<meta[^>]+charset=[\"']?([A-Za-z0-9_-]+)", RegexOption.IGNORE_CASE).find(head)?.groupValues?.get(1)
        val name = CHARSETS[(fromHeader ?: fromMeta)?.lowercase()] ?: "UTF-8"
        return Charset.forName(name)
    }

    /** 厳密に読む（不正な並びは、文字化けにせず断る）。 */
    internal fun decode(bytes: ByteArray, charset: Charset, strict: Boolean = true): String =
        try {
            val action = if (strict) CodingErrorAction.REPORT else CodingErrorAction.REPLACE
            charset.newDecoder()
                .onMalformedInput(action)
                .onUnmappableCharacter(action)
                .decode(ByteBuffer.wrap(bytes))
                .toString()
        } catch (_: CharacterCodingException) {
            throw IllegalArgumentException(t("ページの文字コードを読めません（UTF-8・Shift_JIS・EUC-JP だけ読めます）", "Cannot read the character encoding of the page (only UTF-8, Shift_JIS and EUC-JP can be read)"))
        }
}
