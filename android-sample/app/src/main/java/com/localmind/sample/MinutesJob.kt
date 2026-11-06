package com.localmind.sample

import android.content.Context
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import uniffi.lm_ffi.SpokenSegment
import uniffi.lm_llm_ffi.LlmException
import uniffi.lm_llm_ffi.MinutesEngine
import uniffi.lm_llm_ffi.MinutesEntry
import uniffi.lm_llm_ffi.MinutesReport
import uniffi.lm_llm_ffi.SpokenLine

/**
 * LLM のモデル（GGUF）。1 GB を超えるので、アプリには同梱しない。端末の `files/llm/` に置く:
 * `adb push qwen2.5-1.5b-instruct-q4_k_m.gguf /data/local/tmp/` のあと
 * `adb shell run-as com.localmind.sample cp /data/local/tmp/<ファイル> files/llm/`。
 * ファイルの SHA-256 は、取得元（Hugging Face）が公開している値。
 */
private const val LLM_DIR = "llm"
private const val LLM_FILE = "qwen2.5-1.5b-instruct-q4_k_m.gguf"
private const val LLM_SHA256 = "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e"

/** 文字起こしから議事録を作る。重いので、別スレッドから呼ぶこと。実行中かどうかと中止は、プロセス全体で持つ。 */
object MinutesJob {
    private val running = AtomicBoolean(false)
    @Volatile private var engine: MinutesEngine? = null
    @Volatile private var cancelRequested = false

    val isRunning: Boolean get() = running.get()

    fun modelFile(context: Context): File = File(File(context.filesDir, LLM_DIR), LLM_FILE)

    private val lock = Any()

    /** 中止を求める。エンジンの解放と競合しないよう、同じロックで行う。 */
    fun cancel() {
        synchronized(lock) {
            cancelRequested = true
            engine?.cancel()
        }
    }

    /**
     * 録音 `id` の議事録を作って保存する。すでに実行中なら null。モデルが無ければ、置き方を伝える例外を投げる。
     * 保存した議事録は、`RecordingStore.exportMinutes` で書き出せる。
     */
    fun run(context: Context, id: String, segments: List<SpokenSegment>): MinutesReport? {
        val model = modelFile(context)
        check(model.isFile) { "議事録のモデルがありません。${model.parentFile?.name}/${model.name} を端末のアプリ領域へ置いてください" }
        if (!running.compareAndSet(false, true)) return null
        cancelRequested = false
        try {
            val opened = MinutesEngine.open(
                model.absolutePath, LLM_SHA256,
                Runtime.getRuntime().availableProcessors().coerceIn(1, 4).toUInt(),
            )
            synchronized(lock) {
                engine = opened
                if (cancelRequested) opened.cancel() // 読み込み中に届いた中止も有効にする
            }
            try {
                val lines = segments.map { SpokenLine(it.startMs, it.endMs, it.text) }
                return opened.extractAndSave(File(context.filesDir, "recordings").absolutePath, id, lines)
            } finally {
                synchronized(lock) {
                    engine = null
                    opened.close()
                }
            }
        } finally {
            running.set(false)
        }
    }

    /** 画面に出す文面。根拠の発話の時刻（mm:ss）を、各項目の末尾に付ける。 */
    fun format(report: MinutesReport): String = buildString {
        section("要約", report.summary)
        section("決定事項", report.decisions)
        section("やること", report.actionItems)
        section("未解決の論点", report.openIssues)
        if (report.rejected > 0u) append("\n（根拠を確認できなかった ${report.rejected} 件は載せていません）")
    }.trim()

    private fun StringBuilder.section(title: String, items: List<MinutesEntry>) {
        if (items.isEmpty()) return
        append("■ ").append(title).append('\n')
        for (e in items) {
            append("・").append(e.text)
            val who = listOfNotNull(e.assignee?.let { "担当: $it" }, e.due?.let { "期限: $it" })
            if (who.isNotEmpty()) append("（").append(who.joinToString("、")).append("）")
            append(" [").append(e.citations.joinToString(", ") { clock(it.toLong()) }).append("]\n")
        }
        append('\n')
    }

    private fun clock(ms: Long): String {
        val sec = ms / 1000
        return String.format(java.util.Locale.US, "%02d:%02d", sec / 60, sec % 60)
    }

    fun userMessage(e: Throwable, fallback: String): String = when (e) {
        is LlmException.InvalidInput -> e.message ?: fallback
        is LlmException.Integrity -> "モデルのファイルが壊れています。置き直してください"
        is LlmException.Cancelled -> "中止しました"
        is LlmException.Engine -> e.message ?: fallback
        is IllegalStateException -> e.message ?: fallback
        else -> fallback
    }
}
