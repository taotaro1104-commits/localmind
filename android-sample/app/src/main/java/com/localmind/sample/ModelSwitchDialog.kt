package com.localmind.sample

import android.app.Activity
import android.app.AlertDialog
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.widget.LinearLayout
import android.widget.ScrollView

/**
 * 取得済みの AI モデルを切り替える画面。モデルを押すと、そのモデルを使う（次の質問・議事録から）。
 * 使っていないモデルは「削除」で消せる（容量を空ける。もう一度ダウンロードできる）。
 * 「モデルを追加」は、新しいモデルのダウンロード（候補から選ぶか URL を入れる）へ進む。
 * 議事録の作成・質問への回答・ダウンロードの最中は、モデルを差し替えないよう、切り替えない。
 */
internal fun showModelSwitcher(activity: Activity, onChanged: () -> Unit = {}) {
    if (ModelDownloader.isRunning || ModelGate.isBusy()) {
        AlertDialog.Builder(activity)
            .setMessage("モデルを使う処理（議事録の作成・質問・ダウンロード）が終わってから、切り替えてください。")
            .setPositiveButton("閉じる", null).show()
        return
    }
    ModelSwitcher(activity, onChanged).show()
}

private class ModelSwitcher(private val activity: Activity, private val onChanged: () -> Unit) {
    private val ui = Ui(activity)
    private val note = ui.text(13f, Palette.pass)
    private val rows = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    private lateinit var dialog: AlertDialog

    fun show() {
        val box = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(ui.px(20), ui.px(12), ui.px(20), ui.px(4))
            addView(ui.text(14f, Palette.muted).apply { text = "使うモデルを選びます。大きいモデルほど精度が高く、遅く、メモリを使います。" }, ui.lp())
            addView(rows, ui.lp(top = 12))
            addView(note, ui.lp(top = 8))
        }
        dialog = AlertDialog.Builder(activity)
            .setTitle("AI モデルを切り替える")
            .setView(ScrollView(activity).apply { addView(box) })
            .setPositiveButton("モデルを追加") { _, _ -> offerModelDownload(activity) { onChanged() } }
            .setNegativeButton("閉じる", null)
            .show()
        refresh()
    }

    private fun refresh() {
        rows.removeAllViews()
        val active = ModelStore.active(activity)?.file?.name
        val models = ModelStore.installed(activity)
        if (models.isEmpty()) {
            rows.addView(ui.text(15f, Palette.muted).apply { text = "取得済みのモデルがありません。「モデルを追加」から取得してください。" })
            return
        }
        models.forEach { rows.addView(row(it, it.file.name == active), ui.lp(top = 8)) }
    }

    private fun row(m: InstalledModel, isActive: Boolean) = LinearLayout(activity).apply {
        gravity = Gravity.CENTER_VERTICAL
        setPadding(ui.px(14), ui.px(12), ui.px(8), ui.px(12))
        background = GradientDrawable().apply {
            setColor(Palette.white)
            setStroke(ui.px(if (isActive) 2 else 1), if (isActive) Palette.pass else Palette.line)
        }
        isClickable = true
        contentDescription = "${m.label}。" + if (isActive) "使用中" else "押すと、このモデルに切り替える"
        setOnClickListener { if (!isActive) switchTo(m) }
        addView(
            LinearLayout(activity).apply {
                orientation = LinearLayout.VERTICAL
                addView(ui.text(15f, Palette.ink, bold = true).apply { text = m.label })
                val state = if (isActive) "使用中 ・ " else ""
                addView(ui.text(13f, if (isActive) Palette.pass else Palette.muted).apply { text = "$state${m.megabytes} MB" }, ui.lp(top = 2))
            },
            LinearLayout.LayoutParams(0, -2, 1f),
        )
        if (!isActive) {
            addView(
                ui.text(14f, Palette.fail).apply {
                    text = "削除"
                    setPadding(ui.px(14), ui.px(10), ui.px(10), ui.px(10))
                    isClickable = true
                    contentDescription = "${m.label}を削除する"
                    setOnClickListener { confirmRemove(m) }
                },
            )
        }
    }

    private fun switchTo(m: InstalledModel) {
        if (ModelDownloader.isRunning || ModelGate.isBusy()) {
            note.text = "モデルを使う処理の最中は、切り替えられません"
            note.setTextColor(Palette.fail)
            return
        }
        try {
            ModelStore.switchTo(activity, m.file.name)
            AskJob.release() // 質問のために開いたままの前のモデルは閉じる（メモリを返す）
            note.text = "「${m.label}」に切り替えました。次の質問・議事録から使います"
            note.setTextColor(Palette.pass)
            onChanged()
        } catch (e: IllegalStateException) {
            note.text = e.message
            note.setTextColor(Palette.fail)
        }
        refresh()
    }

    private fun confirmRemove(m: InstalledModel) {
        AlertDialog.Builder(activity)
            .setTitle("「${m.label}」を削除しますか？")
            .setMessage("約 ${m.megabytes} MB の空きができます。元に戻せません（必要なら、もう一度ダウンロードできます）。")
            .setPositiveButton("削除") { _, _ ->
                try {
                    AskJob.release() // 開いたままだと、削除しても容量が空かない
                    ModelStore.remove(activity, m.file.name)
                    note.text = "削除しました"
                    note.setTextColor(Palette.pass)
                } catch (e: IllegalStateException) {
                    note.text = e.message
                    note.setTextColor(Palette.fail)
                }
                refresh()
                onChanged()
            }
            .setNegativeButton("キャンセル", null)
            .show()
    }
}
