package com.localmind.sample

import android.content.Context
import uniffi.lm_llm_ffi.AnswerReport
import uniffi.lm_llm_ffi.LlmException
import uniffi.lm_llm_ffi.MinutesEngine
import uniffi.lm_llm_ffi.SearchPlan
import uniffi.lm_llm_ffi.WebSource

/** 質問への答え方。 */
enum class AskMode {
    /** 端末の中のモデルだけで答える（通信しない）。 */
    LOCAL,

    /** 必要と判断したときだけ、利用者の確認のうえで検索語を Wikipedia へ送り、その結果をもとに答える。 */
    AUTO,
}

/**
 * 回答と、その作り方。`searchedQuery` は Wikipedia へ送った検索語（送っていなければ null。送れなかった場合も、
 * 送ろうとした語を入れる）。`note` は、検索しなかった・できなかったときの断り。
 */
data class AskOutcome(val report: AnswerReport, val searchedQuery: String?, val note: String?)

/**
 * 資料なしの質問応答。モデルは、画面が見えているあいだ開いたままにして（読み込みと検証に時間がかかるため）、
 * 画面が隠れたら [release] で閉じる。議事録の作成（[MinutesJob]）とは [ModelGate] で同時に動かさない。
 */
object AskJob {
    private val lock = Any()
    private var engine: MinutesEngine? = null // lock で守る
    private var openedSha: String? = null // lock で守る
    private var closeAfterRun = false // lock で守る

    @Volatile private var cancelRequested = false

    val isRunning: Boolean get() = ModelGate.isHeldBy(ModelGate.ASK)

    /** 中止を求める。実行中の生成と検索を止める。 */
    fun cancel() {
        synchronized(lock) {
            cancelRequested = true
            engine?.cancel()
        }
        WebSearch.cancel()
    }

    /** モデルを閉じる。実行中なら、中止して、終わったあとに閉じる。 */
    fun release() {
        synchronized(lock) {
            if (isRunning) {
                closeAfterRun = true
                cancelRequested = true
                engine?.cancel()
            } else {
                closeLocked()
            }
        }
        WebSearch.cancel()
    }

    private fun closeLocked() {
        engine?.close()
        engine = null
        openedSha = null
        closeAfterRun = false
    }

    private fun engineFor(model: ActiveModel): MinutesEngine {
        synchronized(lock) {
            engine?.let { if (openedSha == model.sha256) return it else closeLocked() }
        }
        val opened = MinutesEngine.open(
            model.file.absolutePath, model.sha256,
            Runtime.getRuntime().availableProcessors().coerceIn(1, 4).toUInt(),
        )
        synchronized(lock) {
            engine = opened
            openedSha = model.sha256
            if (cancelRequested) opened.cancel() // 読み込み中に届いた中止も有効にする
        }
        return opened
    }

    /**
     * 質問に答える。すでに実行中なら null。モデルが無い・議事録を作成中のときは、理由を伝える例外を投げる。
     * `onStatus` は、いまの段階を（任意のスレッドから）伝える。`confirmSearch` は、検索語を Wikipedia へ送る前に、
     * 利用者の確認を取る（false なら送らず、端末の中だけで答える。回答するスレッドを止めてよい）。
     * 重いので、別スレッドから呼ぶこと。
     */
    fun ask(
        context: Context,
        question: String,
        mode: AskMode,
        onStatus: (String) -> Unit,
        confirmSearch: (String) -> Boolean,
    ): AskOutcome? {
        if (!ModelGate.tryEnter(ModelGate.ASK)) {
            check(!ModelGate.isHeldBy(ModelGate.MINUTES)) { "議事録を作成中です。終わってからお試しください" }
            return null
        }
        try {
            // 使うモデルは、ゲートに入ってから決める（切り替え・削除と食い違わないように）
            val model = checkNotNull(ModelStore.active(context)) { "AI モデルがありません。先にダウンロードしてください" }
            synchronized(lock) {
                cancelRequested = false
                // 生成が走っていないときに届いた前回の中止が、今回の生成を止めないようにする
                engine?.resetCancel()
            }
            onStatus("モデルを読み込んでいます…")
            val e = engineFor(model)
            val plan = if (mode == AskMode.AUTO) plan(e, question, onStatus) else null
            return finish(e, question, plan, onStatus, confirmSearch)
        } finally {
            synchronized(lock) {
                if (closeAfterRun) closeLocked()
            }
            ModelGate.leave(ModelGate.ASK)
        }
    }

    /** 検索が要るかの判断。判断できなかったとき（出力を読めないなど）は、検索せずに答える（null）。 */
    private fun plan(e: MinutesEngine, question: String, onStatus: (String) -> Unit): SearchPlan? {
        onStatus("Web 検索が必要か考えています…")
        return try {
            e.planSearch(question)
        } catch (ex: LlmException.Engine) {
            null
        }
    }

    private fun finish(
        e: MinutesEngine,
        question: String,
        plan: SearchPlan?,
        onStatus: (String) -> Unit,
        confirmSearch: (String) -> Boolean,
    ): AskOutcome {
        var note: String? = null
        var sources = emptyList<WebSource>()
        var query = plan?.takeIf { it.needSearch }?.query
        if (query != null) {
            val allowed = confirmSearch(query)
            if (cancelRequested) throw LlmException.Cancelled()
            if (!allowed) {
                query = null
                note = "検索語は送らず、端末の中の知識だけで答えました。"
            }
        }
        if (query != null) {
            onStatus("Wikipedia を「$query」で検索しています…")
            try {
                sources = WebSearch.search(query)
                if (sources.isEmpty()) note = "検索で資料が見つからなかったので、端末の中の知識だけで答えました。"
            } catch (ex: WebSearchException) {
                if (cancelRequested) throw LlmException.Cancelled()
                note = "${ex.message}。端末の中の知識だけで答えました。"
            }
        }
        if (cancelRequested) throw LlmException.Cancelled()
        onStatus("回答を作っています…")
        val report = e.answer(question, sources)
        return AskOutcome(report, query, note)
    }
}
