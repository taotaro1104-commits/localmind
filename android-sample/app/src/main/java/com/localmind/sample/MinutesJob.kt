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

/** 文字起こしから議事録を作る。重いので、別スレッドから呼ぶこと。実行中かどうかと中止は、プロセス全体で持つ。 */
object MinutesJob {
    private val running = AtomicBoolean(false)
    @Volatile private var engine: MinutesEngine? = null
    @Volatile private var cancelRequested = false

    val isRunning: Boolean get() = running.get()

    /** 使うモデル（[ModelStore]）。無ければ null。 */
    fun activeModel(context: Context): ActiveModel? = ModelStore.active(context)

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
        if (!running.compareAndSet(false, true)) return null
        if (!ModelGate.tryEnter(ModelGate.MINUTES)) {
            running.set(false)
            error(t("質問に回答中です。終わってからお試しください", "A question is being answered. Please try again when it is done"))
        }
        cancelRequested = false
        try {
            // 使うモデルは、ゲートに入ってから決める（切り替え・削除と食い違わないように）
            val model = checkNotNull(activeModel(context)) { t("議事録のモデルがありません。先にダウンロードしてください", "There is no model for the minutes. Please download one first") }
            AskJob.release() // 質問画面で開いたままのモデルは閉じる（モデルを同時に 2 つ開かない）
            val opened = MinutesEngine.open(
                model.file.absolutePath, model.sha256,
                Runtime.getRuntime().availableProcessors().coerceIn(1, 4).toUInt(),
            )
            synchronized(lock) {
                engine = opened
                if (cancelRequested) opened.cancel() // 読み込み中に届いた中止も有効にする
            }
            try {
                val lines = segments.map { SpokenLine(it.startMs, it.endMs, it.text) }
                val report = opened.extractAndSave(File(context.filesDir, "recordings").absolutePath, id, lines)
                runCatching { ModelStore.prune(context) } // このモデルで作れたので、前のモデルは要らない
                return report
            } finally {
                synchronized(lock) {
                    engine = null
                    opened.close()
                }
            }
        } finally {
            running.set(false)
            ModelGate.leave(ModelGate.MINUTES)
        }
    }

    /** 画面に出す文面。根拠の発話の時刻（mm:ss）を、各項目の末尾に付ける。 */
    fun format(report: MinutesReport): String = buildString {
        section(t("要約", "Summary"), report.summary)
        section(t("決定事項", "Decisions"), report.decisions)
        section(t("やること", "Action items"), report.actionItems)
        section(t("未解決の論点", "Open issues"), report.openIssues)
        if (report.rejected > 0u) append(t("\n（根拠を確認できなかった ${report.rejected} 件は載せていません）", "\n(${report.rejected} items whose source could not be confirmed are not shown)"))
    }.trim()

    private fun StringBuilder.section(title: String, items: List<MinutesEntry>) {
        if (items.isEmpty()) return
        append("■ ").append(title).append('\n')
        for (e in items) {
            append(t("・", "- ")).append(e.text)
            val who = listOfNotNull(e.assignee?.let { t("担当: $it", "Owner: $it") }, e.due?.let { t("期限: $it", "Due: $it") })
            if (who.isNotEmpty()) append(t("（", " (")).append(who.joinToString(t("、", ", "))).append(t("）", ")"))
            append(" [").append(e.citations.joinToString(", ") { clock(it.toLong()) }).append("]\n")
        }
        append('\n')
    }

    private fun clock(ms: Long): String {
        val sec = ms / 1000
        return String.format(java.util.Locale.US, "%02d:%02d", sec / 60, sec % 60)
    }

    fun userMessage(e: Throwable, fallback: String): String = when (e) {
        is LlmException.InvalidInput -> ErrorText.localize(e.msg) ?: fallback
        is LlmException.Integrity -> t("モデルのファイルが壊れています。置き直してください", "The model file is damaged. Please put it in place again")
        is LlmException.Cancelled -> t("中止しました", "Stopped")
        is LlmException.Engine -> ErrorText.localize(e.msg) ?: fallback
        is IllegalStateException -> e.message ?: fallback
        else -> fallback
    }
}
