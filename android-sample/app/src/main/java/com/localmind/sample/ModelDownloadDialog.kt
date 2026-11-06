package com.localmind.sample

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.net.ConnectivityManager
import android.text.InputType
import android.view.View
import android.view.WindowManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.Spinner
import kotlin.concurrent.thread

private const val PROGRESS_STEPS = 1000

/** 回線が従量制（モバイル通信など）か。判断できなければ true にして、注意を出す。 */
private fun isMetered(context: Context): Boolean {
    val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager ?: return true
    return cm.isActiveNetworkMetered
}

private fun megabytes(bytes: Long) = bytes / (1024 * 1024)

/**
 * 議事録のモデルが無いときの案内。候補（コンボボックス）から選ぶか、URL を直接入れて選ぶ。
 * 利用者が同意したときだけ接続する。取得できたら `onReady` を呼ぶ。このアプリでネットワークに出るのは、ここだけ。
 */
internal fun offerModelDownload(activity: Activity, onReady: () -> Unit) {
    if (ModelDownloader.isRunning) return
    val ui = Ui(activity)
    val labels = MODEL_CATALOG.map { it.label }
    val spinner = Spinner(activity).apply {
        adapter = ArrayAdapter(activity, android.R.layout.simple_spinner_dropdown_item, labels)
        contentDescription = "モデルの候補"
    }
    val urlField = EditText(activity).apply {
        hint = "または URL を直接入力（https://…）"
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
        isSingleLine = true
        contentDescription = "モデルの URL"
    }
    val shaField = EditText(activity).apply {
        hint = "SHA-256（任意。入れると照合します）"
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        isSingleLine = true
        visibility = View.GONE
        contentDescription = "SHA-256"
    }
    val note = ui.text(13f, Palette.muted)
    val warn = ui.text(13f, Palette.fail, bold = true)
    fun refreshNote() {
        val custom = urlField.text.isNotBlank()
        spinner.isEnabled = !custom
        spinner.alpha = if (custom) 0.4f else 1f
        shaField.visibility = if (custom) View.VISIBLE else View.GONE
        warn.visibility = if (custom) View.VISIBLE else View.GONE
        warn.text = "注意: 直接入れた URL のモデルは、このアプリの中で読み込まれます。悪意のあるファイルだと、端末の録音や文字起こしを盗まれるおそれがあります。信頼できる取得元だけを指定してください。"
        note.text = buildString {
            append("議事録の作成は端末の中だけで行います。接続するのは、ダウンロードのときだけです")
            append("（録音や文字起こしは送りません）。")
            if (custom) {
                append(NL2).append("SHA-256 を入れると、取得元が示す値と照合します（安全の保証ではありません）。入れないと、取得した内容をこのモデルとして記録します。")
                append("Qwen2.5 以外のモデルは、議事録の質が下がることがあります。")
            }
            if (isMetered(activity)) append(NL2).append("今の回線は従量制の可能性があります。Wi-Fi をおすすめします。")
        }
    }
    urlField.addTextChangedListener(object : android.text.TextWatcher {
        override fun afterTextChanged(s: android.text.Editable?) = refreshNote()
        override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
        override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
    })
    spinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
        override fun onItemSelected(p: AdapterView<*>?, v: View?, pos: Int, id: Long) = refreshNote()
        override fun onNothingSelected(p: AdapterView<*>?) {}
    }
    refreshNote()
    val box = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(ui.px(24), ui.px(12), ui.px(24), ui.px(4))
        addView(ui.text(15f, Palette.ink).apply { text = "議事録を作るには、AI モデルが必要です。候補から選びます。" }, ui.lp())
        addView(spinner, ui.lp(top = 8))
        addView(urlField, ui.lp(top = 12))
        addView(shaField, ui.lp(top = 4))
        addView(warn, ui.lp(top = 12))
        addView(note, ui.lp(top = 12))
    }
    AlertDialog.Builder(activity)
        .setTitle("モデルをダウンロード")
        .setView(ScrollView(activity).apply { addView(box) })
        .setPositiveButton("ダウンロード", null)
        .setNegativeButton("やめる", null)
        .create()
        .also { dialog ->
            dialog.setOnShowListener {
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                    val source = try {
                        if (urlField.text.isNotBlank()) ModelSource.custom(urlField.text.toString(), shaField.text.toString())
                        else ModelSource.of(MODEL_CATALOG[spinner.selectedItemPosition])
                    } catch (e: ModelDownloadException) {
                        urlField.error = e.message
                        return@setOnClickListener
                    }
                    if (urlField.text.isNotBlank()) {
                        confirmCustom(activity, source.url.host) { dialog.dismiss(); runDownload(activity, source, onReady) }
                    } else {
                        dialog.dismiss()
                        runDownload(activity, source, onReady)
                    }
                }
            }
        }
        .show()
}

private fun runDownload(activity: Activity, source: ModelSource, onReady: () -> Unit) {
    val ui = Ui(activity)
    val resumed = ModelDownloader.partialBytes(activity, source)
    val label = ui.text(15f, Palette.ink).apply {
        text = if (resumed > 0) "続きから再開します（${megabytes(resumed)}MB 取得済み）…" else "接続しています…"
    }
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
            ModelDownloader.download(activity.applicationContext, source) { done, total ->
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

/** 直接入れた URL のモデルは、同意（チェック）してから取得する。接続先のホスト名を見せる。 */
private fun confirmCustom(activity: Activity, host: String, onConfirmed: () -> Unit) {
    val ui = Ui(activity)
    val agree = android.widget.CheckBox(activity).apply { text = "リスクを理解しました" }
    val box = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(ui.px(24), ui.px(12), ui.px(24), ui.px(4))
        addView(ui.text(15f, Palette.ink).apply {
            text = "「$host」からモデルを取得します。" + NL2 +
                "このモデルは、録音や文字起こしを扱うこのアプリの中で読み込まれます。中身を確かめられないファイルは、" +
                "悪意があると、データの流出や、アプリの乗っ取りにつながるおそれがあります。" + NL2 +
                "前に使っていたモデルは、新しいモデルで議事録を作れたあとに削除します。"
        }, ui.lp())
        addView(agree, ui.lp(top = 12))
    }
    val dialog = AlertDialog.Builder(activity)
        .setTitle("この取得元を信頼しますか？")
        .setView(ScrollView(activity).apply { addView(box) })
        .setPositiveButton("取得する") { _, _ -> onConfirmed() }
        .setNegativeButton("やめる", null)
        .create()
    dialog.setOnShowListener {
        val ok = dialog.getButton(AlertDialog.BUTTON_POSITIVE)
        ok.isEnabled = false
        agree.setOnCheckedChangeListener { _, checked -> ok.isEnabled = checked }
    }
    dialog.show()
}
