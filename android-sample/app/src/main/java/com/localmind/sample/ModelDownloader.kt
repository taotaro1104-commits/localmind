package com.localmind.sample

import android.content.Context
import android.os.StatFs
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

/** ダウンロードに失敗した理由。文面はそのまま画面に出す（URL や端末のパスは入れない）。 */
/** `field` は、入力の誤りがどの欄のものか（"url" / "sha"）。取得中の失敗では null。 */
class ModelDownloadException(message: String, val field: String? = null) : Exception(message)

/**
 * 取得するモデル。候補（[ModelSpec]）なら大きさと SHA-256 が固定されていて、接続先も取得元に限る。
 * 利用者が入れた URL（[custom]）は、https で、サイズが分かるものだけ。SHA-256 を入れたときは照合し、
 * 入れなかったときは、取得した内容の SHA-256 を「このモデル」として記録する（取得元を信頼できる場合だけ使うこと）。
 */
class ModelSource private constructor(
    val url: URL,
    val expectedBytes: Long?,
    val expectedSha256: String?,
    val fixedFileName: String?,
    val trustedHostsOnly: Boolean,
) {
    companion object {
        fun of(spec: ModelSpec) = ModelSource(URL(spec.url), spec.bytes, spec.sha256, spec.fileName, true)

        /** 利用者が入れた URL。https 以外・認証情報つき・書式の誤りは拒否する。SHA-256 は小文字の 16 進 64 文字（任意）。 */
        fun custom(urlText: String, sha256: String?): ModelSource {
            val url = try { URL(urlText.trim()) } catch (_: Exception) { throw ModelDownloadException("URL の書式が正しくありません", "url") }
            if (url.protocol != "https" || url.userInfo != null || url.host.isNullOrEmpty()) {
                throw ModelDownloadException("URL は https:// で始まるものだけ指定できます", "url")
            }
            val sha = sha256?.trim()?.lowercase()?.takeIf { it.isNotEmpty() }
            if (sha != null && !ModelStore.isValidSha(sha)) {
                throw ModelDownloadException("SHA-256 は 16 進 64 文字で入力してください", "sha")
            }
            return ModelSource(url, null, sha, null, false)
        }
    }
}

/**
 * モデル（GGUF）を取得する。リダイレクトは https だけ辿り（候補は取得元のホストだけ）、サイズの上限を守り、
 * GGUF の形式（先頭の印）と、SHA-256 を確かめてから置く。途中で止まっても、続きから再開できる。
 */
object ModelDownloader {
    private const val MAX_REDIRECTS = 5
    private const val SPARE_BYTES = 200L * 1024 * 1024
    private const val MIN_BYTES = 1024L * 1024
    /** lm-llm が読み込める最大サイズ（8GiB）と同じ。 */
    private const val MAX_BYTES = 8L * 1024 * 1024 * 1024
    // 候補の取得で使われるホストだけ（本体と、ファイルの配信先）。
    private val HOSTS = listOf("huggingface.co", "cdn.hf.co", "xethub.hf.co")

    private val running = AtomicBoolean(false)
    /** 実行中の取得の中止フラグ（取得ごとに作る。前の取得への中止が、次の取得に持ち越されない）。 */
    @Volatile private var stopFlag: AtomicBoolean? = null

    val isRunning: Boolean get() = running.get()

    fun cancel() {
        stopFlag?.set(true)
    }

    internal fun isAllowedHost(host: String?): Boolean {
        val h = host?.lowercase() ?: return false
        return HOSTS.any { h == it || h.endsWith(".$it") }
    }

    /** リダイレクト先として辿ってよいか（https・認証情報なし。候補は、許可したホストだけ）。 */
    internal fun isSafeRedirect(url: URL, trustedOnly: Boolean = true): Boolean =
        url.protocol == "https" && url.userInfo == null && !url.host.isNullOrEmpty() &&
            (!trustedOnly || isAllowedHost(url.host))

    /** 公開ホストからのリダイレクトで、LAN・自分自身（loopback・link-local・私設アドレス）へ向かわせない。 */
    internal fun isLocalAddress(host: String): Boolean = try {
        java.net.InetAddress.getAllByName(host).any {
            it.isAnyLocalAddress || it.isLoopbackAddress || it.isLinkLocalAddress || it.isSiteLocalAddress ||
                (it.address.size == 16 && (it.address[0].toInt() and 0xfe) == 0xfc) // fc00::/7
        }
    } catch (_: Exception) {
        true // 解決できないものは、近づかない
    }

    /** 取りかけのファイル名は、取得元の URL ごとに決める（別のモデルの続きにしない）。 */
    private fun partName(source: ModelSource): String {
        val key = MessageDigest.getInstance("SHA-256").digest(source.url.toString().toByteArray())
        return "dl-" + key.take(6).joinToString("") { "%02x".format(it) } + ".part"
    }

    fun partFile(context: Context, source: ModelSource) = File(ModelStore.dir(context), partName(source))

    /** 再開できる取りかけのバイト数。SHA-256 が分からないモデルは再開しないので 0。 */
    fun partialBytes(context: Context, source: ModelSource): Long =
        if (source.expectedSha256 == null) 0L else partFile(context, source).takeIf { it.isFile }?.length() ?: 0L

    /**
     * モデルを取得して使うモデルにする。重いので別スレッドから呼ぶこと。すでに実行中なら false。
     * `onProgress(取得済み, 全体)` は、進むたびに呼ばれる。中止は [cancel]。
     */
    fun download(context: Context, source: ModelSource, onVerify: () -> Unit = {}, onProgress: (Long, Long) -> Unit): Boolean {
        if (!running.compareAndSet(false, true)) return false
        val stop = AtomicBoolean(false)
        stopFlag = stop
        try {
            val dir = ModelStore.dir(context)
            if (!dir.isDirectory && !dir.mkdirs()) throw ModelDownloadException("保存先を作れません")
            val part = partFile(context, source)
            // ほかの取りかけは掃除する（容量のため）。SHA-256 が分からないモデルは、再開せず最初から取る。
            dir.listFiles()?.filter { it.name.startsWith("dl-") && it.name.endsWith(".part") && it != part }?.forEach { it.delete() }
            if (source.expectedSha256 == null) part.delete()
            val total = fetch(source, part, dir, stop, onProgress)
            if (part.length() != total) throw ModelDownloadException("ダウンロードが途中で終わりました。もう一度お試しください")
            checkGguf(part)
            onVerify()
            val sha = sha256(part, stop)
            val expected = source.expectedSha256
            if (expected != null && sha != expected) {
                part.delete()
                throw ModelDownloadException("ダウンロードしたファイルが想定と違います（SHA-256 が一致しません）")
            }
            val name = source.fixedFileName ?: ("custom-" + sha.take(12) + ".gguf")
            val target = File(dir, name)
            if (!part.renameTo(target)) throw ModelDownloadException("モデルを保存できませんでした")
            ModelStore.activate(context, target, sha)
            return true
        } finally {
            stopFlag = null
            running.set(false)
        }
    }

    private fun open(source: ModelSource, start: Long): HttpURLConnection {
        var url = source.url
        repeat(MAX_REDIRECTS + 1) {
            if (!isSafeRedirect(url, source.trustedHostsOnly)) throw ModelDownloadException("想定外の接続先のため中止しました")
            // 入力された URL のホストは利用者の意思。そこから別のホストへ飛ばされた先が、LAN や自端末なら断る。
            if (!source.trustedHostsOnly && !url.host.equals(source.url.host, ignoreCase = true) && isLocalAddress(url.host)) {
                throw ModelDownloadException("ローカルネットワークへのリダイレクトのため中止しました")
            }
            val c = url.openConnection() as HttpURLConnection
            c.instanceFollowRedirects = false
            c.connectTimeout = 15_000
            c.readTimeout = 30_000
            c.setRequestProperty("Accept-Encoding", "identity")
            c.setRequestProperty("User-Agent", "LocalMind")
            if (start > 0) c.setRequestProperty("Range", "bytes=$start-")
            val code = c.responseCode
            if (code in 300..399) {
                val next = c.getHeaderField("Location")
                c.disconnect()
                if (next.isNullOrBlank()) throw ModelDownloadException("接続先を解決できません")
                url = try { URL(url, next) } catch (_: Exception) { throw ModelDownloadException("接続先を解決できません") }
            } else {
                return c
            }
        }
        throw ModelDownloadException("リダイレクトが多すぎます")
    }

    /** 全体のサイズ（バイト）を、応答から求める。分からない・範囲外なら失敗にする。 */
    private fun totalBytes(conn: HttpURLConnection, code: Int, start: Long, source: ModelSource): Long {
        val total = if (code == 206) {
            val range = conn.getHeaderField("Content-Range").orEmpty()
            if (!range.startsWith("bytes $start-")) return -1L
            range.substringAfterLast('/').toLongOrNull() ?: -1L
        } else {
            conn.contentLengthLong
        }
        if (total !in MIN_BYTES..MAX_BYTES) return -1L
        if (source.expectedBytes != null && total != source.expectedBytes) return -1L
        return total
    }

    private fun fetch(source: ModelSource, part: File, dir: File, stop: AtomicBoolean, onProgress: (Long, Long) -> Unit): Long {
        var start = if (part.isFile) part.length() else 0L
        if (source.expectedBytes != null && start > source.expectedBytes) { part.delete(); start = 0L }
        if (source.expectedBytes != null && start == source.expectedBytes) return start // 取り終わっている（検証だけ行う）
        val conn = try { open(source, start) } catch (e: ModelDownloadException) { throw e } catch (_: javax.net.ssl.SSLException) {
            throw ModelDownloadException("安全な接続を確立できませんでした（回線や証明書に問題があるかもしれません）")
        } catch (_: IOException) {
            throw ModelDownloadException("ネットワークに接続できません")
        }
        try {
            val code = conn.responseCode
            if (code == 416) {
                // すべて取得済み（検証の途中で中止したあと）なら、そのまま検証へ進む。SHA-256 が分からなければ破棄する。
                if (source.expectedSha256 != null && start > 0) return start
                part.delete()
                throw ModelDownloadException("取りかけのファイルを破棄しました。もう一度お試しください")
            }
            if (code != 200 && code != 206) throw ModelDownloadException("ダウンロードできません（サーバーの応答 $code）")
            if (code == 200) start = 0L // 再開を受け付けない場合は、最初から
            val total = totalBytes(conn, code, start, source)
            if (total < 0) {
                if (code == 206) part.delete()
                throw ModelDownloadException("サイズを確認できないか、想定と違うため中止しました")
            }
            if (StatFs(dir.path).availableBytes < total - start + SPARE_BYTES) {
                throw ModelDownloadException("空き容量が足りません（あと約 ${(total - start + SPARE_BYTES) / (1024 * 1024)}MB 必要です）")
            }
            conn.inputStream.use { input ->
                FileOutputStream(part, start > 0).use { out ->
                    val buf = ByteArray(256 * 1024)
                    var done = start
                    onProgress(done, total)
                    while (true) {
                        if (stop.get()) throw ModelDownloadException("中止しました（取りかけは残します）")
                        val n = try { input.read(buf) } catch (_: IOException) {
                            throw ModelDownloadException("通信が途切れました。もう一度押すと続きから再開します")
                        }
                        if (n < 0) break
                        done += n
                        if (done > total) { part.delete(); throw ModelDownloadException("想定より大きいため中止しました") }
                        out.write(buf, 0, n)
                        onProgress(done, total)
                    }
                }
            }
            return total
        } finally {
            conn.disconnect()
        }
    }

    /** 先頭が GGUF の印か（形式の違うファイルを、置く前に断る）。 */
    private fun checkGguf(file: File) {
        val head = ByteArray(4)
        val n = file.inputStream().use { it.read(head) }
        if (n != 4 || String(head, Charsets.ISO_8859_1) != "GGUF") {
            file.delete()
            throw ModelDownloadException("GGUF 形式のファイルではありません")
        }
    }

    private fun sha256(file: File, stop: AtomicBoolean): String {
        val md = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buf = ByteArray(1024 * 1024)
            while (true) {
                if (stop.get()) throw ModelDownloadException("中止しました（取りかけは残します）")
                val n = input.read(buf)
                if (n < 0) break
                md.update(buf, 0, n)
            }
        }
        return md.digest().joinToString("") { "%02x".format(it) }
    }
}
