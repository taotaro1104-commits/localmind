package com.localmind.sample

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.text.InputFilter
import android.text.InputType
import android.text.util.Linkify
import android.view.Gravity
import android.view.View
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import uniffi.lm_llm_ffi.maxQuestionChars

private const val WIKI_PREFIX = "https://ja.wikipedia.org/"

/** 質問画面の見た目。状態の変更はすべて UI スレッドから呼ぶこと。 */
class AskScreen(
    private val context: Context,
    private val onAsk: (question: String, mode: AskMode) -> Unit,
    private val onCancel: () -> Unit,
    private val onSwitchModel: () -> Unit,
) {
    private val ui = Ui(context)
    private val px = ui::px

    var mode = AskMode.LOCAL
        private set
    private var running = false

    private val banner = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Palette.ink)
        addView(ui.text(28f, Palette.white, bold = true).apply { text = "AI に質問" })
        addView(ui.text(15f, Palette.white).apply { text = "資料は読み込まず、質問だけに答えます" }, ui.lp(top = 4))
    }
    private val modelChip = ui.text(14f, Palette.white, bold = true).apply {
        setPadding(px(14), px(8), px(14), px(8))
        background = GradientDrawable().apply { setColor(Color.TRANSPARENT); setStroke(px(1), Palette.white) }
        isClickable = true
        setOnClickListener { if (!running) onSwitchModel() }
    }

    private val tiles = mapOf(
        AskMode.LOCAL to tile("端末の中だけで答える", "通信しません。質問は端末の外に出ません。新しい出来事は知らないことがあります。"),
        AskMode.AUTO to tile(
            "必要なら Wikipedia を検索して答える",
            "検索が必要と判断したときだけ、AI が作った短い検索語を、送る前にお見せして確認します。確認できた語だけを Wikipedia（日本語版）へ送ります。",
        ),
    )

    private val question = EditText(context).apply {
        hint = "質問を入力"
        textSize = 16f
        setTextColor(Palette.ink)
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        minLines = 3
        gravity = Gravity.TOP
        filters = arrayOf(InputFilter.LengthFilter(maxQuestionChars().toInt()))
        setPadding(px(14), px(12), px(14), px(12))
        background = GradientDrawable().apply { setColor(Palette.white); setStroke(px(1), Palette.line) }
        contentDescription = "質問"
    }

    private val action = ui.button("質問する", Palette.ink) { if (running) onCancel() else submit() }
    private val status = ui.text(14f, Palette.muted).apply {
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        visibility = View.GONE
    }

    private val route = ui.text(14f, Palette.white, bold = true).apply { setPadding(px(12), px(6), px(12), px(6)) }
    private val answer = ui.text(17f, Palette.ink).apply { setTextIsSelectable(true) }
    private val note = ui.text(14f, Palette.running)
    private val sources = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val result = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        visibility = View.GONE
        addView(route, ui.lp(wrap = true))
        addView(answer, ui.lp(top = 14))
        addView(note, ui.lp(top = 10))
        addView(sources, ui.lp(top = 14))
        addView(
            ui.text(13f, Palette.muted).apply { text = "AI の回答は誤りを含むことがあります。重要なことは、ご自身で確かめてください。" },
            ui.lp(top = 16),
        )
    }

    private val body = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(ui.text(15f, Palette.ink, bold = true).apply { text = "答え方" })
        AskMode.entries.forEach { addView(tiles.getValue(it), ui.lp(top = 8)) }
        addView(question, ui.lp(top = 20))
        addView(action, ui.lp(top = 12))
        addView(status, ui.lp(top = 12))
        addView(result, ui.lp(top = 24))
    }

    private val scroll = ScrollView(context).apply {
        setBackgroundColor(Palette.paper)
        isFillViewport = true
        isFocusableInTouchMode = true
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            addView(banner)
            addView(body)
        })
        setOnApplyWindowInsetsListener { _, insets ->
            @Suppress("DEPRECATION")
            banner.setPadding(px(20), insets.systemWindowInsetTop + px(28), px(20), px(24))
            @Suppress("DEPRECATION")
            body.setPadding(px(20), px(20), px(20), insets.systemWindowInsetBottom + px(32))
            insets
        }
    }
    val view: View get() = scroll

    init {
        banner.addView(modelChip, ui.lp(top = 14, wrap = true))
        AskMode.entries.forEach { m -> tiles.getValue(m).setOnClickListener { if (!running) select(m) } }
        select(AskMode.LOCAL)
    }

    /** 答え方の選択肢。選んでいるものは、太い枠と濃い題名にする。 */
    private fun tile(title: String, detail: String) = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        isClickable = true
        setPadding(px(14), px(12), px(14), px(12))
        addView(ui.text(16f, Palette.ink, bold = true).apply { text = title })
        addView(ui.text(13f, Palette.muted).apply { text = detail }, ui.lp(top = 4))
        contentDescription = "$title。$detail"
    }

    private fun select(m: AskMode) {
        mode = m
        tiles.forEach { (k, tile) ->
            val on = k == m
            tile.background = GradientDrawable().apply {
                setColor(Palette.white)
                setStroke(px(if (on) 2 else 1), if (on) Palette.ink else Palette.line)
            }
            tile.isSelected = on
        }
    }

    /** いま使っているモデルの名前を出す（押すと切り替え画面）。無ければ null。 */
    fun setModel(label: String?) {
        modelChip.text = if (label == null) "AI モデル: 未取得 ▾" else "AI モデル: $label ▾"
        modelChip.contentDescription = (if (label == null) "AI モデルは未取得。" else "いまの AI モデルは $label。") + "押すと切り替える"
    }

    private fun submit() {
        val q = question.text.toString().trim()
        if (q.isEmpty()) {
            question.error = "質問を入力してください"
            return
        }
        onAsk(q, mode)
    }

    /** 実行中は、ボタンを「中止」にして、入力と答え方の変更を止める。 */
    fun setRunning(on: Boolean) {
        running = on
        action.text = if (on) "中止" else "質問する"
        action.background = ui.fill(if (on) Palette.fail else Palette.ink)
        action.contentDescription = action.text
        question.isEnabled = !on
        tiles.values.forEach { it.alpha = if (on) 0.5f else 1f }
        if (on) result.visibility = View.GONE
    }

    fun setStatus(text: String?, ok: Boolean = true) {
        status.visibility = if (text == null) View.GONE else View.VISIBLE
        status.text = text.orEmpty()
        status.setTextColor(if (ok) Palette.muted else Palette.fail)
    }

    /** 回答を出す。送った検索語があれば、いちばん上の帯に出す（データが端末の外へ出たかどうかを一目で分けるため）。 */
    fun showAnswer(outcome: AskOutcome) {
        val sent = outcome.searchedQuery
        route.text = if (sent == null) "端末の中だけで作成" else "Wikipedia に「$sent」を送って作成"
        route.background = ui.fill(if (sent == null) Palette.pass else Palette.running)
        answer.text = outcome.report.text
        note.text = outcome.note.orEmpty()
        note.visibility = if (outcome.note == null) View.GONE else View.VISIBLE
        sources.removeAllViews()
        if (outcome.report.sources.isNotEmpty()) {
            sources.addView(ui.text(15f, Palette.ink, bold = true).apply { text = "出典" })
            outcome.report.sources.forEachIndexed { i, s ->
                val cited = (i + 1).toUInt() in outcome.report.cited
                sources.addView(
                    ui.text(14f, Palette.ink, bold = true).apply {
                        text = "[${i + 1}] ${s.title}" + (if (cited) "" else "（回答では使っていません）")
                        setTextIsSelectable(true)
                    },
                    ui.lp(top = 8),
                )
                // リンクにするのは URL の行だけ（題名は Web 由来なので、リンクにしない）。日本語版 Wikipedia だけ。
                sources.addView(
                    ui.text(13f, Palette.muted).apply {
                        text = s.url
                        setTextIsSelectable(true)
                        if (s.url.startsWith(WIKI_PREFIX)) Linkify.addLinks(this, Linkify.WEB_URLS)
                    },
                    ui.lp(top = 2),
                )
            }
        }
        result.visibility = View.VISIBLE
        scroll.post { scroll.smoothScrollTo(0, result.top) }
    }

    fun clearFocus() {
        question.clearFocus()
    }

    val questionView: TextView get() = question
}
