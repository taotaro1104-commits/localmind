package com.localmind.sample

import android.app.Activity
import android.app.AlertDialog
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.view.WindowInsetsController
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import uniffi.lm_llm_ffi.LlmException

/**
 * 資料なしで質問に答える画面。既定は、端末の LLM だけで答える（通信しない）。
 * 利用者が選んだときだけ、必要に応じて Wikipedia を検索し、その結果をもとに答える。
 */
class AskActivity : Activity() {
    private lateinit var screen: AskScreen
    private val debuggable by lazy { applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0 }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.hide()
        screen = AskScreen(this, ::ask, ::cancel, ::switchModel)
        setContentView(screen.view)
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            window.insetsController?.setSystemBarsAppearance(0, WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS)
        }
    }

    override fun onResume() {
        super.onResume()
        refreshModel()
    }

    private fun refreshModel() = screen.setModel(ModelStore.activeLabel(applicationContext))

    private fun switchModel() = showModelSwitcher(this) { refreshModel() }

    /** 画面が隠れたら、モデルを閉じる（メモリを返す。実行中なら中止する）。 */
    override fun onStop() {
        super.onStop()
        AskJob.release()
    }

    @Volatile private var confirmDialog: AlertDialog? = null

    /** 検索語を送る前に、利用者に見せて確かめる（回答のスレッドから呼ぶ。答えが出るまで待つ）。 */
    private fun confirmSearch(query: String): Boolean {
        val latch = CountDownLatch(1)
        val allow = AtomicBoolean(false)
        runOnUiThread {
            if (isDestroyed) {
                latch.countDown()
                return@runOnUiThread
            }
            confirmDialog = AlertDialog.Builder(this)
                .setTitle("Wikipedia に送りますか？")
                .setMessage("この検索語を、Wikipedia（日本語版）へ送ります。" + NL2 + "「$query」" + NL2 + "送らないと、端末の中の知識だけで答えます。")
                .setPositiveButton("送る") { _, _ -> allow.set(true) }
                .setNegativeButton("送らない", null)
                .setOnDismissListener { confirmDialog = null; latch.countDown() }
                .show()
        }
        latch.await()
        return allow.get()
    }

    override fun onDestroy() {
        confirmDialog?.dismiss() // 確認を待っている回答のスレッドを、止めたままにしない
        super.onDestroy()
    }

    private fun cancel() {
        confirmDialog?.dismiss()
        AskJob.cancel()
        screen.setStatus("中止しています…")
    }

    private fun ask(question: String, mode: AskMode) {
        if (AskJob.isRunning) return
        if (MinutesJob.isRunning || SpeechJob.isRunning || RecorderState.active) {
            screen.setStatus("録音・文字起こし・議事録の作成が終わってからお試しください", ok = false)
            return
        }
        if (ModelStore.active(applicationContext) == null) {
            offerModelDownload(this) { refreshModel(); ask(question, mode) }
            return
        }
        hideKeyboard()
        screen.setRunning(true)
        screen.setStatus("準備しています…")
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val context = applicationContext
        thread(name = "lm-ask") {
            val result = runCatching {
                AskJob.ask(
                    context, question, mode,
                    onStatus = { text -> runOnUiThread { if (!isDestroyed) screen.setStatus(text) } },
                    confirmSearch = ::confirmSearch,
                )
            }
            runOnUiThread {
                window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                if (isDestroyed) return@runOnUiThread
                screen.setRunning(false)
                result.onSuccess { outcome ->
                    if (outcome == null) {
                        screen.setStatus("すでに質問に答えています", ok = false)
                    } else {
                        screen.setStatus(null)
                        screen.showAnswer(outcome)
                    }
                }.onFailure { fail(it) }
            }
        }
    }

    private fun fail(e: Throwable) {
        if (debuggable) android.util.Log.w("LMSample", "ask failure: $e", e)
        if (e is LlmException.Cancelled) {
            screen.setStatus("中止しました")
            return
        }
        val msg = MinutesJob.userMessage(e, "回答を作れませんでした")
        screen.setStatus(msg, ok = false)
        // モデルが読み込めないときは、モデルを選び直せるようにする
        if (e is LlmException.Integrity || e is LlmException.Engine) {
            AlertDialog.Builder(this).setTitle("回答を作れませんでした").setMessage(msg)
                .setPositiveButton("閉じる", null)
                .setNeutralButton("モデルを変更") { _, _ -> switchModel() }
                .show()
        }
    }

    private fun hideKeyboard() {
        screen.clearFocus()
        (getSystemService(INPUT_METHOD_SERVICE) as? InputMethodManager)
            ?.hideSoftInputFromWindow(screen.questionView.windowToken, 0)
    }
}
