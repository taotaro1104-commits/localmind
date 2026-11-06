package com.localmind.sample

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.net.ConnectivityManager
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.ProgressBar
import kotlin.concurrent.thread

private const val PROGRESS_STEPS = 1000

/** 回線が従量制（モバイル通信など）か。判断できなければ true にして、注意を出す。 */
private fun isMetered(context: Context): Boolean {
    val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager ?: return true
    return cm.isActiveNetworkMetered
}

private fun megabytes(bytes: Long) = bytes / (1024 * 1024)

/**
 * 議事録のモデルが無いときの案内。利用者が同意したときだけ、取得元（Hugging Face）へ接続して取得する。
 * 取得できたら `onReady` を呼ぶ。このアプリでネットワークに出るのは、ここだけ。
 */
internal fun offerModelDownload(activity: Activity, onReady: () -> Unit) {
    if (ModelDownloader.isRunning) return
    val resumed = ModelDownloader.partialBytes(activity)
    val note = buildString {
        append("議事録を作るには、AI モデル（Qwen2.5、約 1.1GB）が必要です。")
        append(NL2).append("モデルは端末に保存し、議事録の作成は端末の中だけで行います。")
        append("ダウンロードのために、この 1 回だけ huggingface.co へ接続します（録音や文字起こしは送りません）。")
        if (isMetered(activity)) append(NL2).append("今の回線は従量制の可能性があります。Wi-Fi をおすすめします。")
        if (resumed > 0) append(NL2).append("前回の続き（${megabytes(resumed)}MB 取得済み）から再開します。")
    }
    AlertDialog.Builder(activity)
        .setTitle("モデルをダウンロード")
        .setMessage(note)
        .setPositiveButton(if (resumed > 0) "続きから" else "ダウンロード") { _, _ -> runDownload(activity, onReady) }
        .setNegativeButton("やめる", null)
        .show()
}

private fun runDownload(activity: Activity, onReady: () -> Unit) {
    val ui = Ui(activity)
    val label = ui.text(15f, Palette.ink).apply { text = "接続しています…" }
    val bar = ProgressBar(activity, null, android.R.attr.progressBarStyleHorizontal).apply { max = PROGRESS_STEPS }
    val box = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(ui.px(24), ui.px(16), ui.px(24), ui.px(8))
        addView(label, ui.lp())
        addView(bar, ui.lp(top = 12))
    }
    val dialog = AlertDialog.Builder(activity)
        .setTitle("ダウンロード中")
        .setView(box)
        .setCancelable(false)
        .setNegativeButton("中止", null)
        .show()
    dialog.getButton(AlertDialog.BUTTON_NEGATIVE).setOnClickListener {
        ModelDownloader.cancel()
        label.text = "中止しています…"
    }
    activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    var lastStep = -1
    thread(name = "lm-model-download") {
        val result = runCatching {
            ModelDownloader.download(activity.applicationContext) { done, total ->
                val step = (done * PROGRESS_STEPS / total).toInt()
                if (step != lastStep) { // 画面の更新は、0.1% 進んだときだけ
                    lastStep = step
                    activity.runOnUiThread {
                        bar.progress = step
                        label.text = "${megabytes(done)}MB / ${megabytes(total)}MB"
                    }
                }
            }
        }
        activity.runOnUiThread {
            activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            if (activity.isDestroyed) return@runOnUiThread
            dialog.dismiss()
            result.onSuccess {
                if (it) onReady() else AlertDialog.Builder(activity).setMessage("すでにダウンロード中です")
                    .setPositiveButton("閉じる", null).show()
            }.onFailure {
                val msg = (it as? ModelDownloadException)?.message ?: "ダウンロードできませんでした"
                AlertDialog.Builder(activity).setTitle("ダウンロードできませんでした").setMessage(msg)
                    .setPositiveButton("閉じる", null).show()
            }
        }
    }
}
