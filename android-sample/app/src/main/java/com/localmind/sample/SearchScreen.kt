package com.localmind.sample

import android.content.Context
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import uniffi.lm_ffi.Citation
import uniffi.lm_ffi.DocumentInfo
import uniffi.lm_ffi.WorkspaceReport

/** 検索画面の見た目。状態の変更はすべて UI スレッドから呼ぶこと。 */
class SearchScreen(
    private val context: Context,
    private val onSearch: (String) -> Unit,
    onAdd: (title: String, text: String) -> Unit,
    onRemove: (id: String, title: String) -> Unit,
    onPickFile: () -> Unit,
) {
    private val dp = context.resources.displayMetrics.density
    private fun px(v: Int) = (v * dp).toInt()

    private val title = text(28f, Palette.white, bold = true).apply { text = "資料を検索" }
    private val status = text(15f, Palette.white)
    private val banner = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(title)
        addView(status, margins(top = 4))
    }

    private val input = EditText(context).apply {
        hint = "質問を入力"
        textSize = 17f
        setTextColor(Palette.ink)
        setHintTextColor(Palette.muted)
        inputType = InputType.TYPE_CLASS_TEXT
        imeOptions = EditorInfo.IME_ACTION_SEARCH
        isSingleLine = true
        background = underline(Palette.ink, px(2))
        setPadding(0, px(12), 0, px(12))
        setOnEditorActionListener { _, action, _ ->
            (action == EditorInfo.IME_ACTION_SEARCH).also { if (it) submit() }
        }
    }
    private val button = TextView(context).apply {
        text = "検索"
        textSize = 16f
        setTextColor(Palette.white)
        setTypeface(Typeface.create("sans-serif-medium", Typeface.BOLD))
        gravity = Gravity.CENTER
        setPadding(px(22), px(12), px(22), px(12))
        contentDescription = "検索を実行する"
        setOnClickListener { submit() }
    }
    private val examples = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val results = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val docs = DocsPanel(context, onAdd, onRemove, onPickFile)
    private val body = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(
            LinearLayout(context).apply {
                gravity = Gravity.BOTTOM
                addView(input, LinearLayout.LayoutParams(0, -2, 1f))
                addView(button, margins(left = 12))
            },
            margins(top = 4),
        )
        addView(examples, margins(top = 18))
        addView(results, margins(top = 8))
        addView(docs.view, margins(top = 36))
    }
    private val scroll = ScrollView(context).apply {
        setBackgroundColor(Palette.paper)
        isFillViewport = true
        addView(
            LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(banner)
                addView(body)
            },
        )
        // API 35 では画面が端まで広がる。バーの下に隠れないよう、上下の余白を取る。
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
        setState(Palette.running, "モデルを読み込んでいます（初回は時間がかかります）")
        enable(false)
        docs.enable(false)
        SAMPLE_QUESTIONS.forEach { examples.addView(exampleRow(it)) }
        examples.addView(
            text(13f, Palette.muted).apply { text = "例の質問を押すと、そのまま検索します" },
            margins(top = 8),
        )
    }

    /** 準備ができた、または資料が増減した。 */
    fun documents(list: List<DocumentInfo>) {
        docs.setDocuments(list)
        setState(Palette.pass, "${list.size} 件の資料から探せます。端末の外には送りません。")
        enable(true)
        docs.enable(true)
    }

    /** 資料の追加・削除などの状況（入力中の欄は触らない）。 */
    fun note(message: String, ok: Boolean) = docs.report(message, ok)

    /** 追加に成功したので、入力欄を空にする。 */
    fun clearForm() = docs.clearForm()

    /** 資料の追加・削除を実行している間は、操作を止める。 */
    fun busy(on: Boolean) {
        enable(!on)
        docs.enable(!on)
    }

    fun failed(message: String) {
        setState(Palette.fail, "うまくいきませんでした")
        enable(false)
        results.removeAllViews()
        results.addView(text(14f, Palette.fail).apply { text = message })
    }

    fun searching() {
        enable(false)
        results.removeAllViews()
        results.addView(text(15f, Palette.running).apply { text = "探しています…" }, margins(top = 16))
    }

    fun show(query: String, r: WorkspaceReport) {
        enable(true)
        results.removeAllViews()
        if (!r.found || r.citations.isEmpty()) {
            results.addView(notFound(query), margins(top = 16))
            return
        }
        results.addView(heading("根拠になる資料（近い順に ${r.citations.size} 件）"), margins(top = 20))
        r.citations.forEachIndexed { i, c -> results.addView(hitRow(i + 1, c, last = i == r.citations.lastIndex)) }
    }

    private fun submit() {
        val q = input.text.toString().trim()
        if (q.isEmpty() || !button.isEnabled) return
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(input.windowToken, 0)
        onSearch(q)
    }

    private fun enable(on: Boolean) {
        button.isEnabled = on
        input.isEnabled = on
        button.background = GradientDrawable().apply { setColor(if (on) Palette.ink else Palette.muted) }
        button.alpha = if (on) 1f else 0.6f
    }

    private fun setState(color: Int, message: String) {
        banner.setBackgroundColor(color)
        status.text = message
    }

    private fun exampleRow(q: String) = text(15f, Palette.pass).apply {
        text = q
        setPadding(0, px(10), 0, px(10))
        background = underline(Palette.line, px(1))
        isClickable = true
        contentDescription = "例の質問「$q」で検索する"
        setOnClickListener {
            if (button.isEnabled) {
                input.setText(q)
                onSearch(q)
            }
        }
    }

    private fun notFound(query: String) = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(px(16), px(14), px(16), px(16))
        background = GradientDrawable().apply {
            setColor(Palette.white)
            setStroke(px(2), Palette.fail)
        }
        addView(text(20f, Palette.fail, bold = true).apply { text = "資料には見つかりません" })
        addView(
            text(14f, Palette.muted).apply { text = "「$query」に答えられる記述は、登録した資料にありませんでした。" },
            margins(top = 6),
        )
    }

    private fun heading(label: String) = text(18f, Palette.ink, bold = true).apply {
        text = label
        setPadding(0, 0, 0, px(8))
        background = underline(Palette.ink, px(2))
    }

    private fun hitRow(rank: Int, c: Citation, last: Boolean) =
        LinearLayout(context).apply {
            gravity = Gravity.TOP
            setPadding(0, px(14), 0, px(14))
            if (!last) background = underline(Palette.line, px(1))
            addView(
                text(15f, Palette.white, bold = true).apply {
                    text = "$rank"
                    gravity = Gravity.CENTER
                    background = GradientDrawable().apply {
                        shape = GradientDrawable.OVAL
                        setColor(if (rank == 1) Palette.pass else Palette.muted)
                    }
                },
                LinearLayout.LayoutParams(px(26), px(26)),
            )
            addView(
                LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(text(16f, Palette.ink, bold = true).apply { text = c.title })
                    addView(text(13f, Palette.pass).apply { text = where(c) }, margins(top = 2))
                    addView(text(14f, Palette.muted).apply { text = c.excerpt }, margins(top = 6))
                },
                LinearLayout.LayoutParams(0, -2, 1f).apply { marginStart = px(14) },
            )
        }

    /** 出典の位置。見出しの並び（文書の題名と同じ先頭は省く）と、原本の行番号。 */
    private fun where(c: Citation): String {
        val path = c.headingPath.dropWhile { it == c.title }.joinToString(" › ")
        val lines = if (c.startLine == c.endLine) "${c.startLine} 行目" else "${c.startLine}〜${c.endLine} 行目"
        return if (path.isEmpty()) lines else "$path ・ $lines"
    }

    private fun underline(color: Int, thickness: Int) = LayerDrawable(
        arrayOf(GradientDrawable().apply { setColor(color) }, GradientDrawable().apply { setColor(Palette.paper) }),
    ).apply { setLayerInset(1, 0, 0, 0, thickness) }

    private fun text(size: Float, color: Int, bold: Boolean = false) = TextView(context).apply {
        textSize = size
        setTextColor(color)
        if (bold) setTypeface(Typeface.create("sans-serif-medium", Typeface.BOLD))
        setLineSpacing(0f, 1.2f)
    }

    private fun margins(top: Int = 0, left: Int = 0, height: Int = ViewGroup.LayoutParams.WRAP_CONTENT) =
        LinearLayout.LayoutParams(
            if (left == 0) ViewGroup.LayoutParams.MATCH_PARENT else ViewGroup.LayoutParams.WRAP_CONTENT,
            if (height > 0) px(height) else height,
        ).apply {
            topMargin = px(top)
            marginStart = px(left)
        }
}
