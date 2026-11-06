package com.localmind.sample

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.pm.ApplicationInfo
import android.net.ConnectivityManager
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.util.Log
import android.view.View
import android.view.WindowManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.CheckBox
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

/** モデルを選ぶ入力欄（候補のコンボボックス・URL・SHA-256）と、状況に合わせた注意書き。 */
private class ModelForm(private val activity: Activity) {
    private val ui = Ui(activity)
    val spinner = Spinner(activity).apply {
        adapter = ArrayAdapter(activity, android.R.layout.simple_spinner_dropdown_item, MODEL_CATALOG.map { it.label })
        contentDescription = t("モデルの候補", "Model choices")
    }
    val url = EditText(activity).apply {
        hint = t("または URL を直接入力（https://…）", "Or enter a URL directly (https://…)")
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
        isSingleLine = true
        contentDescription = t("モデルの URL。入れると、候補は使いません", "URL of the model. If you enter one, the choices are not used")
    }
    val sha = EditText(activity).apply {
        hint = t("SHA-256（任意。入れると照合します）", "SHA-256 (optional; it is checked if you enter it)")
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        isSingleLine = true
        visibility = View.GONE
        contentDescription = "SHA-256"
    }
    private val warn = ui.text(13f, Palette.fail, bold = true)
    private val note = ui.text(13f, Palette.muted)

    val isCustom get() = url.text.isNotBlank()

    /** 入力から取得元を作る。誤りは、該当する欄に出して null。 */
    fun source(): ModelSource? = try {
        if (isCustom) ModelSource.custom(url.text.toString(), sha.text.toString()) else ModelSource.of(MODEL_CATALOG[spinner.selectedItemPosition])
    } catch (e: ModelDownloadException) {
        (if (e.field == "sha") sha else url).error = e.message
        null
    }

    fun refresh() {
        spinner.isEnabled = !isCustom
        spinner.alpha = if (isCustom) 0.4f else 1f
        sha.visibility = if (isCustom) View.VISIBLE else View.GONE
        warn.visibility = if (isCustom) View.VISIBLE else View.GONE
        warn.text = t("注意: 直接入れた URL のモデルは、このアプリの中で読み込まれます。悪意のあるファイルだと、端末の録音や文字起こしを", "Warning: a model from a URL you enter is loaded inside this app. A malicious file could steal your recordings and transcripts.") +
            t("盗まれるおそれがあります。信頼できる取得元だけを指定してください。", "Only specify sources you trust.")
        note.text = buildString {
            append(t("議事録の作成は端末の中だけで行います。接続するのは、ダウンロードのときだけです（録音や文字起こしは送りません）。", "Making minutes happens on the device only. The app connects only to download (recordings and transcripts are never sent)."))
            if (isCustom) {
                append(NL2).append(t("SHA-256 を入れると、取得元が示す値と照合します（安全の保証ではありません）。入れないと、取得した内容をこのモデルとして記録します。", "If you enter a SHA-256, it is checked against the value the source gives (this is not a guarantee of safety). If you leave it empty, the downloaded content is recorded as this model."))
                append(t("Qwen2.5 以外のモデルは、議事録の質が下がることがあります。", "Models other than Qwen2.5 may make worse minutes."))
            }
            if (isMetered(activity)) append(NL2).append(t("今の回線は従量制の可能性があります。Wi-Fi をおすすめします。", "Your connection may be metered. Wi-Fi is recommended."))
        }
    }

    fun view(): View {
        url.addTextChangedListener(object : TextWatcher {
            override fun afterTextChanged(s: Editable?) = refresh()
            override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
            override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
        })
        spinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(p: AdapterView<*>?, v: View?, pos: Int, id: Long) = refresh()
            override fun onNothingSelected(p: AdapterView<*>?) {}
        }
        refresh()
        val box = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(ui.px(24), ui.px(12), ui.px(24), ui.px(4))
            addView(ui.text(15f, Palette.ink).apply { text = t("議事録を作るには、AI モデルが必要です。候補から選びます。", "You need an AI model to make minutes. Choose one from the list.") }, ui.lp())
            addView(spinner, ui.lp(top = 8))
            addView(url, ui.lp(top = 12))
            addView(sha, ui.lp(top = 4))
            addView(warn, ui.lp(top = 12))
            addView(note, ui.lp(top = 12))
        }
        return ScrollView(activity).apply { addView(box) }
    }
}

/**
 * モデルを取得して使うモデルにする案内。候補（コンボボックス）から選ぶか、URL を直接入れる。
 * 利用者が同意したときだけ接続する。取得できたら `onReady` を呼ぶ。このアプリでネットワークに出るのは、ここだけ。
 */
internal fun offerModelDownload(activity: Activity, onReady: () -> Unit) {
    if (ModelDownloader.isRunning) {
        AlertDialog.Builder(activity).setMessage(t("モデルをダウンロード中です。終わるまでお待ちください。", "A model is being downloaded. Please wait until it finishes."))
            .setPositiveButton(t("閉じる", "Close"), null).show()
        return
    }
    val form = ModelForm(activity)
    val dialog = AlertDialog.Builder(activity)
        .setTitle(t("モデルをダウンロード", "Download a model"))
        .setView(form.view())
        .setPositiveButton(t("ダウンロード", "Download"), null)
        .setNegativeButton(t("やめる", "Cancel"), null)
        .create()
    dialog.setOnShowListener {
        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
            val source = form.source() ?: return@setOnClickListener
            val start = { dialog.dismiss(); runDownload(activity, source, onReady) }
            if (form.isCustom) confirmCustom(activity, source.url.host, start) else start()
        }
    }
    dialog.show()
}

/** 直接入れた URL のモデルは、同意（チェック）してから取得する。接続先のホスト名を見せる。 */
private fun confirmCustom(activity: Activity, host: String, onConfirmed: () -> Unit) {
    val ui = Ui(activity)
    val agree = CheckBox(activity).apply { text = t("リスクを理解しました", "I understand the risk") }
    val box = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(ui.px(24), ui.px(12), ui.px(24), ui.px(4))
        addView(ui.text(15f, Palette.ink).apply {
            text = t("「$host」からモデルを取得します。", "The model will be downloaded from “$host”.") + NL2 +
                t("このモデルは、録音や文字起こしを扱うこのアプリの中で読み込まれます。中身を確かめられないファイルは、", "This model is loaded inside this app, which handles recordings and transcripts. A file whose contents you cannot check") +
                t("悪意があると、データの流出や、アプリの乗っ取りにつながるおそれがあります。", "can leak data or take over the app if it is malicious.") + NL2 +
                t("前に使っていたモデルは残します。「モデルを変更」で、いつでも切り替えたり、削除したりできます。", "The previous model is kept. You can switch to it or delete it at any time with “Change model”.")
        }, ui.lp())
        addView(agree, ui.lp(top = 12))
    }
    val dialog = AlertDialog.Builder(activity)
        .setTitle(t("この取得元を信頼しますか？", "Do you trust this source?"))
        .setView(ScrollView(activity).apply { addView(box) })
        .setPositiveButton(t("取得する", "Download")) { _, _ -> onConfirmed() }
        .setNegativeButton(t("やめる", "Cancel"), null)
        .create()
    dialog.setOnShowListener {
        val ok = dialog.getButton(AlertDialog.BUTTON_POSITIVE)
        ok.isEnabled = false
        agree.setOnCheckedChangeListener { _, checked -> ok.isEnabled = checked }
    }
    dialog.show()
}

/** 進み具合を出す小さなダイアログ。中止ボタンつき。 */
private class ProgressDialog(activity: Activity, initial: String) {
    private val ui = Ui(activity)
    val label = ui.text(15f, Palette.ink).apply {
        text = initial
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    val bar = ProgressBar(activity, null, android.R.attr.progressBarStyleHorizontal).apply {
        max = PROGRESS_STEPS
        contentDescription = t("ダウンロードの進み具合", "Download progress")
    }
    val dialog: AlertDialog = AlertDialog.Builder(activity)
        .setTitle(t("ダウンロード中", "Downloading"))
        .setView(LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(ui.px(24), ui.px(16), ui.px(24), ui.px(8))
            addView(label, ui.lp())
            addView(bar, ui.lp(top = 12))
        })
        .setCancelable(false)
        .setNegativeButton(t("中止", "Stop"), null)
        .show()
        .also { d ->
            d.getButton(AlertDialog.BUTTON_NEGATIVE).setOnClickListener {
                ModelDownloader.cancel()
                label.text = t("中止しています…", "Stopping…")
            }
        }
}

private fun showFailure(activity: Activity, error: Throwable) {
    val debuggable = activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
    if (debuggable) Log.w("LMSample", "model download failure: $error", error)
    val msg = (error as? ModelDownloadException)?.message ?: t("ダウンロードできませんでした", "Could not download")
    AlertDialog.Builder(activity).setTitle(t("ダウンロードできませんでした", "Could not download")).setMessage(msg)
        .setPositiveButton(t("閉じる", "Close"), null).show()
}

private fun runDownload(activity: Activity, source: ModelSource, onReady: () -> Unit) {
    val resumed = ModelDownloader.partialBytes(activity, source)
    val progress = ProgressDialog(
        activity,
        if (resumed > 0) t("続きから再開します（${megabytes(resumed)}MB 取得済み）…", "Resuming (${megabytes(resumed)} MB already downloaded)…") else t("接続しています…", "Connecting…"),
    )
    activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    thread(name = "lm-model-download") {
        var lastStep = -1
        val result = runCatching {
            ModelDownloader.download(
                activity.applicationContext, source,
                onVerify = { activity.runOnUiThread { progress.label.text = t("内容を確かめています…", "Checking the contents…") } },
            ) { done, total ->
                val step = (done * PROGRESS_STEPS / total).toInt()
                if (step != lastStep) { // 画面の更新は、0.1% 進んだときだけ
                    lastStep = step
                    activity.runOnUiThread {
                        progress.bar.progress = step
                        progress.label.text = "${megabytes(done)}MB / ${megabytes(total)}MB"
                    }
                }
            }
        }
        activity.runOnUiThread {
            activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            runCatching { progress.dialog.dismiss() } // 画面が破棄されていても、ウィンドウを残さない
            if (activity.isDestroyed) return@runOnUiThread
            result.onSuccess {
                if (it) onReady() else AlertDialog.Builder(activity).setMessage(t("すでにダウンロード中です", "Already downloading"))
                    .setPositiveButton(t("閉じる", "Close"), null).show()
            }.onFailure { showFailure(activity, it) }
        }
    }
}
