package com.localmind.sample

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Process
import android.provider.OpenableColumns
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import uniffi.lm_ffi.DocFormat

/** 取り込む資料（題名・本文・形式）。 */
class Imported(val title: String, val text: String, val format: DocFormat)

/**
 * 共有シートとファイル選択から来た資料を、取り込める形にする。外から来る入力は信頼しない:
 * - 共有の種類（MIME）は許可したものだけ。ファイルは `content://` だけで、読む権限が実際に付与されている
 *   ものだけを読む（他のアプリの provider を、このアプリの権限で読まされないため）。
 * - 読む量と時間に上限を設け、UTF-8 として不正なら断る。
 * 資料の中身の検査（NUL など）と、件数・合計容量の上限は Rust 側が行う。
 */
object Importer {
    /** 読む最大バイト数（Rust 側の 1 文書の上限と同じ）。 */
    const val MAX_BYTES = 256 * 1024
    /** 読み込み全体の制限時間。超えたらストリームを閉じて中断する。 */
    private const val READ_TIMEOUT_SECONDS = 10L
    private const val MAX_TITLE_CHARS = 100
    private val MARKDOWN_EXT = listOf(".md", ".markdown", ".mkd")
    private val ALLOWED_TYPES = setOf("text/plain", "text/markdown", "text/x-markdown")

    private val watchdog = Executors.newSingleThreadScheduledExecutor { r ->
        Thread(r, "lm-import-watchdog").apply { isDaemon = true }
    }

    /** 共有されたテキスト（`EXTRA_TEXT`）。題名は件名、無ければ最初の行。 */
    fun fromText(subject: String?, text: String): Imported {
        val first = text.lineSequence().firstOrNull { it.isNotBlank() }
        return Imported(cleanTitle(subject ?: first), text, DocFormat.TEXT)
    }

    /** 共有・選択されたファイル。読む権限が付与されている `content://` だけを受け付ける。 */
    fun fromUri(context: Context, uri: Uri): Imported {
        requireReadable(context, uri)
        val resolver = context.contentResolver
        val name = displayName(resolver, uri)
        val bytes = resolver.openInputStream(uri)?.use { readWithTimeout(it) } ?: error("ファイルを開けません")
        val markdown = MARKDOWN_EXT.any { name.orEmpty().lowercase().endsWith(it) }
        val stem = name?.substringBeforeLast('.', name)
        return Imported(cleanTitle(stem), decode(bytes), if (markdown) DocFormat.MARKDOWN else DocFormat.TEXT)
    }

    /** 共有の intent から取り込む。テキストは `EXTRA_TEXT`、ファイルは `EXTRA_STREAM`。 */
    fun fromShare(context: Context, intent: Intent): Imported {
        require(intent.type in ALLOWED_TYPES) { "この種類の共有は受け付けません" }
        @Suppress("DEPRECATION")
        val stream = intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
        if (stream != null) return fromUri(context, stream)
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        require(!text.isNullOrBlank()) { "共有された内容が空です" }
        require(text.length <= MAX_BYTES && text.toByteArray().size <= MAX_BYTES) { "共有されたテキストが大きすぎます" }
        return fromText(intent.getStringExtra(Intent.EXTRA_SUBJECT)?.take(MAX_TITLE_CHARS * 2), text)
    }

    /**
     * `content://` で、読む権限がこのアプリに付与されていて、このアプリ自身の provider ではないこと。
     * （送り主が権限を持たない他アプリの provider の URI を渡して、代わりに読ませることを防ぐ。）
     */
    internal fun requireReadable(context: Context, uri: Uri) {
        require(uri.scheme == ContentResolver.SCHEME_CONTENT) { "content:// 以外の場所は読み込めません" }
        require(uri.authority?.startsWith(context.packageName) != true) { "このアプリのデータは読み込めません" }
        val granted = context.checkUriPermission(
            uri, Process.myPid(), Process.myUid(), Intent.FLAG_GRANT_READ_URI_PERMISSION,
        ) == PackageManager.PERMISSION_GRANTED
        require(granted) { "このファイルを読む許可がありません" }
    }

    private fun displayName(resolver: ContentResolver, uri: Uri): String? =
        runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        }.getOrNull()

    /** 制限時間つきで読む。読み出しが戻らないときは、時間が来たらストリームを閉じて中断させる。 */
    private fun readWithTimeout(input: InputStream): ByteArray {
        val guard = watchdog.schedule({ runCatching { input.close() } }, READ_TIMEOUT_SECONDS, TimeUnit.SECONDS)
        try {
            return readLimited(input)
        } catch (e: java.io.IOException) {
            throw IllegalStateException("ファイルの読み込みに時間がかかりすぎたか、読めませんでした")
        } finally {
            guard.cancel(false)
        }
    }

    /** 上限までしか読まない（超えたら断る）。巨大なファイルでメモリを使い切らないため。 */
    internal fun readLimited(input: InputStream): ByteArray {
        val out = ByteArrayOutputStream()
        val buf = ByteArray(8192)
        while (true) {
            val n = input.read(buf)
            if (n < 0) break
            out.write(buf, 0, n)
            require(out.size() <= MAX_BYTES) { "ファイルが大きすぎます（${MAX_BYTES / 1024}KB まで）" }
        }
        return out.toByteArray()
    }

    /** UTF-8 として厳密に読む（不正な並びは、文字化けにせず断る）。 */
    internal fun decode(bytes: ByteArray): String =
        try {
            Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes))
                .toString()
        } catch (e: CharacterCodingException) {
            throw IllegalArgumentException("UTF-8 の文書だけ取り込めます")
        }

    /**
     * 題名に使えない文字（制御・書式・行区切り・単独のサロゲート）を空白にし、文字（コードポイント）単位で
     * 長さを切る。空なら仮の題名。手入力の題名もここを通す。
     */
    fun cleanTitle(raw: String?): String {
        val kept = StringBuilder()
        var count = 0
        raw.orEmpty().codePoints().toArray().forEach { cp ->
            if (count >= MAX_TITLE_CHARS * 2) return@forEach
            val bad = when (Character.getType(cp).toByte()) {
                Character.CONTROL, Character.FORMAT, Character.LINE_SEPARATOR,
                Character.PARAGRAPH_SEPARATOR, Character.SURROGATE, Character.UNASSIGNED,
                -> true
                else -> false
            }
            kept.appendCodePoint(if (bad) ' '.code else cp)
            count++
        }
        val trimmed = kept.toString().trim()
        val cut = trimmed.codePoints().limit(MAX_TITLE_CHARS.toLong()).toArray()
        val s = String(cut, 0, cut.size).trim()
        return s.ifEmpty { "無題の資料" }
    }
}
