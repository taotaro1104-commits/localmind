package com.localmind.sample

import android.content.Context
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import uniffi.lm_ffi.DocFormat
import uniffi.lm_ffi.DocumentInfo

/**
 * 登録した資料の一覧（グループごと）と、資料を足すフォーム。状態の変更は UI スレッドから呼ぶこと。
 * 一覧の上のグループ名を押すと、そのグループだけに切り替える（「すべて」では、グループごとに区切って出す）。
 * 追加するときのグループは、「追加先のグループ」の欄で決める（切り替えたグループが入る）。
 */
class DocsPanel(
    private val context: Context,
    private val onAdd: (title: String, text: String) -> Unit,
    private val onRemove: (id: String, title: String) -> Unit,
    private val onPickFile: () -> Unit,
    private val onAddUrl: (url: String) -> Unit,
    private val onSetGroup: (id: String, group: String) -> Unit,
    private val onRemoveGroup: (group: String, docs: List<DocumentInfo>) -> Unit,
) {
    private val dp = context.resources.displayMetrics.density
    private fun px(v: Int) = (v * dp).toInt()

    private var docs: List<DocumentInfo> = emptyList()

    /** 切り替えているグループ。null ならすべて、空文字なら未分類。 */
    private var selected: String? = null

    private val heading = text(18f, Palette.ink, bold = true)
    private val chips = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val list = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val message = text(14f, Palette.muted)
    private val groupInput = field("追加先のグループ（任意。空なら未分類）", multiline = false).apply {
        filters = arrayOf(android.text.InputFilter.LengthFilter(MAX_GROUP_CHARS))
    }
    private val titleInput = field("題名（例: 出張規程）", multiline = false)
    private val bodyInput = field("本文（Markdown 可。「# 見出し」を付けると、出典に見出しが出ます）", multiline = true)
    private val urlInput = field("https://… のページの URL", multiline = false).apply {
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
    }
    private val addButton = button("資料に追加", "資料に追加する") { submit() }
    private val urlButton = button("URL から取り込む", "URL のページを資料に追加する") { submitUrl() }
    private val form = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        visibility = View.GONE
        addView(titleInput)
        addView(bodyInput, lp(top = 8))
        addView(addButton, lp(top = 14, wrap = true))
    }
    private val urlForm = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        visibility = View.GONE
        addView(urlInput)
        addView(
            text(13f, Palette.muted).apply {
                text = "ページの本文だけを取り込みます（HTML・テキスト・Markdown）。このアプリが通信するのは、いま入れた URL へ 1 回だけです。"
            },
            lp(top = 6),
        )
        addView(urlButton, lp(top = 12, wrap = true))
    }
    private val toggle = opener("＋ 文章を入力して追加", "－ 閉じる", form)
    private val urlToggle = opener("＋ URL から追加", "－ 閉じる", urlForm)

    private val pick = text(16f, Palette.pass, bold = true).apply {
        text = "＋ ファイルから追加（テキスト・Markdown）"
        setPadding(0, px(14), 0, px(14))
        isClickable = true
        contentDescription = "ファイルを選んで資料に追加する"
        setOnClickListener { if (addButton.isEnabled) onPickFile() }
    }

    val view: View = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(heading.apply { setPadding(0, 0, 0, px(8)); background = underline(Palette.ink, px(2)) })
        addView(chips, lp(top = 4))
        addView(list)
        addView(text(15f, Palette.ink, bold = true).apply { text = "資料を追加" }, lp(top = 20))
        addView(groupInput, lp(top = 8))
        addView(pick)
        addView(urlToggle)
        addView(urlForm)
        addView(toggle)
        addView(form)
        addView(message, lp(top = 8))
    }

    /** 検索の範囲。切り替えているグループ（空文字は未分類）。null ならすべて。 */
    fun scope(): String? = selected

    /** 追加先のグループ（前後の空白は除く。空なら未分類）。 */
    fun group(): String = groupInput.text.toString().trim()

    fun setDocuments(docs: List<DocumentInfo>) {
        this.docs = docs
        val kept = DocGroups.keepSelection(DocGroups.sections(docs), selected)
        // 切り替えていたグループが無くなったら、追加先に残った名前も消す（同名のグループが、気づかないうちに復活しない）
        if (selected != null && kept == null) groupInput.setText("")
        selected = kept
        render()
    }

    private fun render() {
        val sections = DocGroups.sections(docs)
        heading.text = "登録した資料（${docs.size} 件）"
        renderChips(sections)
        list.removeAllViews()
        if (docs.isEmpty()) {
            list.addView(text(14f, Palette.muted).apply {
                text = "資料がありません。下から追加してください。"
                setPadding(0, px(12), 0, px(12))
            })
            return
        }
        // 「すべて」のときだけ、グループごとの見出しを出す（1 つのグループを選んだときは、チップが見出しの代わり）
        val showHeaders = selected == null && sections.size > 1
        DocGroups.visible(sections, selected).forEach { section ->
            if (showHeaders) list.addView(sectionHeader(section))
            section.docs.forEachIndexed { i, d -> list.addView(row(d, last = i == section.docs.lastIndex)) }
        }
        // 1 つのグループを切り替えているときは、検索がそのグループに絞られることと、一括削除を出す
        val only = selected?.let { key -> sections.firstOrNull { it.key == key } }
        if (only != null) {
            list.addView(
                text(13f, Palette.muted).apply { text = "検索は「${only.label}」の資料に絞られます（「すべて」に戻すと、全部から探します）。" },
                lp(top = 12),
            )
            list.addView(
                action("「${only.label}」の資料をすべて削除（${only.docs.size} 件）", Palette.fail, "グループ「${only.label}」の資料をすべて削除する") {
                    onRemoveGroup(only.key, only.docs)
                }.apply { setPadding(0, px(14), 0, px(10)) },
            )
        }
    }

    /** 「すべて（n）」と各グループのチップ。選んでいるものは濃く塗る。 */
    private fun renderChips(sections: List<DocSection>) {
        chips.removeAllViews()
        if (sections.size <= 1 && sections.firstOrNull()?.key.isNullOrEmpty()) return // グループが無いときは出さない
        val row = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
        val wrapper = android.widget.HorizontalScrollView(context).apply {
            isHorizontalScrollBarEnabled = false
            addView(row)
        }
        row.addView(chip("すべて（${docs.size}）", selected == null) { select(null) })
        sections.forEach { s -> row.addView(chip("${s.label}（${s.docs.size}）", selected == s.key) { select(s.key) }) }
        chips.addView(wrapper)
    }

    private fun select(key: String?) {
        selected = key
        // 追加先も、切り替えたグループに合わせる（「すべて」は、追加先も空＝未分類に戻す）
        groupInput.setText(key.orEmpty())
        render()
    }

    private fun chip(label: String, on: Boolean, onClick: () -> Unit) = text(14f, if (on) Palette.white else Palette.ink, bold = on).apply {
        text = label
        gravity = Gravity.CENTER
        setPadding(px(14), px(8), px(14), px(8))
        background = GradientDrawable().apply {
            setColor(if (on) Palette.ink else Palette.white)
            setStroke(px(1), Palette.ink)
        }
        isClickable = true
        isSelected = on
        contentDescription = "グループ「$label」に切り替える" + if (on) "（選択中）" else ""
        setOnClickListener { onClick() }
        layoutParams = LinearLayout.LayoutParams(-2, -2).apply { marginEnd = px(8) }
    }

    private fun sectionHeader(s: DocSection) = text(14f, Palette.muted, bold = true).apply {
        text = "${s.label}（${s.docs.size} 件）"
        setPadding(0, px(14), 0, px(6))
        background = underline(Palette.line, px(1))
    }

    fun enable(on: Boolean) {
        for (b in listOf(addButton, urlButton)) {
            b.isEnabled = on
            b.alpha = if (on) 1f else 0.5f
        }
        titleInput.isEnabled = on
        bodyInput.isEnabled = on
        urlInput.isEnabled = on
        groupInput.isEnabled = on
    }

    fun report(note: String, ok: Boolean) {
        message.text = note
        message.setTextColor(if (ok) Palette.pass else Palette.fail)
    }

    /** 追加に成功したら、入力欄を空にする（追加先のグループは、続けて足せるよう残す）。 */
    fun clearForm() {
        titleInput.setText("")
        bodyInput.setText("")
        urlInput.setText("")
    }

    private fun submit() {
        if (!addButton.isEnabled) return
        val title = titleInput.text.toString().trim()
        val body = bodyInput.text.toString()
        when {
            title.isEmpty() -> report("題名を入力してください。", ok = false)
            body.isBlank() -> report("本文を入力してください。", ok = false)
            else -> onAdd(title, body)
        }
    }

    private fun submitUrl() {
        if (!urlButton.isEnabled) return
        val url = urlInput.text.toString().trim()
        if (url.isEmpty()) report("URL を入力してください。", ok = false) else onAddUrl(url)
    }

    private fun row(d: DocumentInfo, last: Boolean) = LinearLayout(context).apply {
        gravity = Gravity.CENTER_VERTICAL
        setPadding(0, px(12), 0, px(12))
        if (!last) background = underline(Palette.line, px(1))
        addView(
            LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(text(16f, Palette.ink, bold = true).apply { text = d.title })
                val kind = if (d.format == DocFormat.MARKDOWN) "Markdown" else "テキスト"
                addView(text(13f, Palette.muted).apply { text = "$kind ・ 断片 ${d.chunks} 件" }, lp(top = 2))
            },
            LinearLayout.LayoutParams(0, -2, 1f),
        )
        addView(action("グループ", Palette.pass, "「${d.title}」のグループを変える") {
            pickGroup(context, d.group, DocGroups.names(docs)) { onSetGroup(d.id, it) }
        })
        addView(action("削除", Palette.fail, "「${d.title}」を削除する") { onRemove(d.id, d.title) })
    }

    private fun action(label: String, color: Int, description: String, onClick: () -> Unit) = text(14f, color).apply {
        text = label
        setPadding(px(12), px(10), 0, px(10))
        isClickable = true
        contentDescription = description
        setOnClickListener { onClick() }
    }

    private fun button(label: String, description: String, onClick: () -> Unit) = text(16f, Palette.white, bold = true).apply {
        text = label
        gravity = Gravity.CENTER
        setPadding(px(22), px(12), px(22), px(12))
        background = GradientDrawable().apply { setColor(Palette.ink) }
        contentDescription = description
        setOnClickListener { onClick() }
    }

    /** フォームを開閉する見出し。 */
    private fun opener(closed: String, open: String, target: View) = text(16f, Palette.pass, bold = true).apply {
        text = closed
        setPadding(0, px(14), 0, px(14))
        isClickable = true
        contentDescription = "$closed（フォームを開閉する）"
        setOnClickListener {
            val show = target.visibility != View.VISIBLE
            target.visibility = if (show) View.VISIBLE else View.GONE
            text = if (show) open else closed
        }
    }

    private fun field(hintText: String, multiline: Boolean) = EditText(context).apply {
        hint = hintText
        textSize = 16f
        setTextColor(Palette.ink)
        setHintTextColor(Palette.muted)
        inputType = InputType.TYPE_CLASS_TEXT or if (multiline) InputType.TYPE_TEXT_FLAG_MULTI_LINE else 0
        if (multiline) {
            minLines = 5
            gravity = Gravity.TOP
        } else {
            isSingleLine = true
        }
        setPadding(px(12), px(12), px(12), px(12))
        background = GradientDrawable().apply {
            setColor(Palette.white)
            setStroke(px(1), Palette.line)
        }
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

    private fun lp(top: Int = 0, wrap: Boolean = false) = LinearLayout.LayoutParams(
        if (wrap) -2 else -1,
        -2,
    ).apply { topMargin = px(top) }
}
