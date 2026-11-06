package com.localmind.sample

import android.content.Context
import android.os.StatFs
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

/** ダウンロードに失敗した理由。文面はそのまま画面に出す（URL や端末のパスは入れない）。 */
class ModelDownloadException(message: String) : Exception(message)

/**
 * 議事録のモデル（GGUF）を取得する。接続先は固定した 1 か所（Hugging Face）だけ。リダイレクトは https かつ許可した
 * ホストへのものだけ辿る。中身は SHA-256 が固定した値と一致するときだけ置く。途中で止まっても、続きから再開できる。
 */
object ModelDownloader {
    private const val URL_TEXT =
        "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/$LLM_FILE"

    /** モデルの大きさ（バイト）。取得元が公開している値。これを超えて読まない。 */
    const val EXPECTED_BYTES = 1_117_320_736L
    private const val MAX_REDIRECTS = 5
    private const val SPARE_BYTES = 200L * 1024 * 1024
    // 実際の取得で使われるホストだけ（本体と、ファイルの配信先）。
    private val HOSTS = listOf("huggingface.co", "cdn.hf.co", "xethub.hf.co")

    private val running = AtomicBoolean(false)
    /** 実行中の取得の中止フラグ（取得ごとに作る。前の取得への中止が、次の取得に持ち越されない）。 */
    @Volatile private var stopFlag: AtomicBoolean? = null

    val isRunning: Boolean get() = running.get()

    fun cancel() {
        stopFlag?.set(true)
    }

    /** ホスト名が許可した取得元（本体、またはそのサブドメイン）か。 */
    internal fun isAllowedHost(host: String?): Boolean {
        val h = host?.lowercase() ?: return false
        return HOSTS.any { h == it || h.endsWith(".$it") }
    }

    internal fun isSafeRedirect(url: URL): Boolean =
        url.protocol == "https" && url.userInfo == null && isAllowedHost(url.host)

    /** 取りかけのファイル（再開に使う）。 */
    fun partFile(context: Context): File = File(MinutesJob.modelFile(context).path + ".part")

    /** 取りかけのバイト数（無ければ 0）。 */
    fun partialBytes(context: Context): Long = partFile(context).takeIf { it.isFile }?.length() ?: 0L

    /**
     * モデルを取得して置く。重いので別スレッドから呼ぶこと。すでに実行中なら false。
     * `onProgress(取得済み, 全体)` は、進むたびに呼ばれる。中止は [cancel]。
     */
    fun download(context: Context, onProgress: (Long, Long) -> Unit): Boolean {
        if (!running.compareAndSet(false, true)) return false
        val stop = AtomicBoolean(false)
        stopFlag = stop
        try {
            val target = MinutesJob.modelFile(context)
            val dir = target.parentFile ?: throw ModelDownloadException("保存先を作れません")
            if (!dir.isDirectory && !dir.mkdirs()) throw ModelDownloadException("保存先を作れません")
            val part = partFile(context)
            if (part.length() > EXPECTED_BYTES) part.delete()
            val have = if (part.isFile) part.length() else 0L
            if (StatFs(dir.path).availableBytes < EXPECTED_BYTES - have + SPARE_BYTES) {
                throw ModelDownloadException("空き容量が足りません（約 1.3GB 必要です）")
            }
            fetch(part, stop, onProgress)
            if (part.length() != EXPECTED_BYTES) throw ModelDownloadException("ダウンロードが途中で終わりました。もう一度お試しください")
            if (sha256(part, stop) != LLM_SHA256) {
                part.delete()
                throw ModelDownloadException("ダウンロードしたファイルが想定と違います。もう一度お試しください")
            }
            if (!part.renameTo(target)) throw ModelDownloadException("モデルを保存できませんでした")
            return true
        } finally {
            stopFlag = null
            running.set(false)
        }
    }

    private fun open(start: Long): HttpURLConnection {
        var url = URL(URL_TEXT)
        repeat(MAX_REDIRECTS + 1) {
            if (!isSafeRedirect(url)) throw ModelDownloadException("想定外の接続先のため中止しました")
            val c = url.openConnection() as HttpURLConnection
            c.instanceFollowRedirects = false
            c.connectTimeout = 15_000
            c.readTimeout = 30_000
            c.setRequestProperty("Accept-Encoding", "identity")
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

    private fun fetch(part: File, stop: AtomicBoolean, onProgress: (Long, Long) -> Unit) {
        var start = if (part.isFile) part.length() else 0L
        if (start == EXPECTED_BYTES) return // 取り終わっている（検証だけ行う）
        val conn = try { open(start) } catch (e: ModelDownloadException) { throw e } catch (_: javax.net.ssl.SSLException) {
            throw ModelDownloadException("安全な接続を確立できませんでした（回線や証明書に問題があるかもしれません）")
        } catch (_: IOException) {
            throw ModelDownloadException("ネットワークに接続できません")
        }
        try {
            val code = conn.responseCode
            if (code == 416) { part.delete(); throw ModelDownloadException("取りかけのファイルを破棄しました。もう一度お試しください") }
            if (code != 200 && code != 206) throw ModelDownloadException("ダウンロードできません（サーバーの応答 $code）")
            if (code == 200) {
                start = 0L // 再開を受け付けない場合は、最初から
                if (StatFs(part.parentFile!!.path).availableBytes < EXPECTED_BYTES + SPARE_BYTES) {
                    throw ModelDownloadException("空き容量が足りません（約 1.3GB 必要です）")
                }
            } else if (conn.getHeaderField("Content-Range")?.startsWith("bytes $start-") != true) {
                part.delete()
                throw ModelDownloadException("サーバーの応答が想定と違います。もう一度お試しください")
            }
            if (conn.contentLengthLong > EXPECTED_BYTES - start) {
                part.delete()
                throw ModelDownloadException("想定より大きいため中止しました")
            }
            conn.inputStream.use { input ->
                java.io.FileOutputStream(part, start > 0).use { out ->
                    val buf = ByteArray(256 * 1024)
                    var total = start
                    onProgress(total, EXPECTED_BYTES)
                    while (true) {
                        if (stop.get()) throw ModelDownloadException("中止しました（取りかけは残します）")
                        val n = try { input.read(buf) } catch (_: IOException) {
                            throw ModelDownloadException("通信が途切れました。もう一度押すと続きから再開します")
                        }
                        if (n < 0) break
                        total += n
                        if (total > EXPECTED_BYTES) { part.delete(); throw ModelDownloadException("想定より大きいため中止しました") }
                        out.write(buf, 0, n)
                        onProgress(total, EXPECTED_BYTES)
                    }
                }
            }
        } finally {
            conn.disconnect()
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
