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

/** 登録した資料の一覧と、資料を足すフォーム。状態の変更は UI スレッドから呼ぶこと。 */
class DocsPanel(
    private val context: Context,
    private val onAdd: (title: String, text: String) -> Unit,
    private val onRemove: (id: String, title: String) -> Unit,
    private val onPickFile: () -> Unit,
) {
    private val dp = context.resources.displayMetrics.density
    private fun px(v: Int) = (v * dp).toInt()

    private val heading = text(18f, Palette.ink, bold = true)
    private val list = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val message = text(14f, Palette.muted)
    private val titleInput = field("題名（例: 出張規程）", multiline = false)
    private val bodyInput = field("本文（Markdown 可。「# 見出し」を付けると、出典に見出しが出ます）", multiline = true)
    private val addButton = text(16f, Palette.white, bold = true).apply {
        text = "資料に追加"
        gravity = Gravity.CENTER
        setPadding(px(22), px(12), px(22), px(12))
        background = GradientDrawable().apply { setColor(Palette.ink) }
        contentDescription = "資料に追加する"
        setOnClickListener { submit() }
    }
    private val form = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        visibility = View.GONE
        addView(titleInput)
        addView(bodyInput, lp(top = 8))
        addView(addButton, lp(top = 14, wrap = true))
    }
    private val toggle = text(16f, Palette.pass, bold = true).apply {
        text = "＋ 資料を追加する"
        setPadding(0, px(14), 0, px(14))
        isClickable = true
        contentDescription = "資料を追加するフォームを開閉する"
        setOnClickListener {
            val open = form.visibility != View.VISIBLE
            form.visibility = if (open) View.VISIBLE else View.GONE
            text = if (open) "－ 閉じる" else "＋ 資料を追加する"
        }
    }

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
        addView(list)
        addView(pick)
        addView(toggle)
        addView(form)
        addView(message, lp(top = 8))
    }

    fun setDocuments(docs: List<DocumentInfo>) {
        heading.text = "登録した資料（${docs.size} 件）"
        list.removeAllViews()
        if (docs.isEmpty()) {
            list.addView(text(14f, Palette.muted).apply {
                text = "資料がありません。下から追加してください。"
                setPadding(0, px(12), 0, px(12))
            })
        }
        docs.forEachIndexed { i, d -> list.addView(row(d, last = i == docs.lastIndex)) }
    }

    fun enable(on: Boolean) {
        addButton.isEnabled = on
        addButton.alpha = if (on) 1f else 0.5f
        titleInput.isEnabled = on
        bodyInput.isEnabled = on
    }

    fun report(note: String, ok: Boolean) {
        message.text = note
        message.setTextColor(if (ok) Palette.pass else Palette.fail)
    }

    /** 追加に成功したら、入力欄を空にする。 */
    fun clearForm() {
        titleInput.setText("")
        bodyInput.setText("")
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
        addView(
            text(14f, Palette.fail).apply {
                text = "削除"
                setPadding(px(12), px(10), 0, px(10))
                isClickable = true
                contentDescription = "「${d.title}」を削除する"
                setOnClickListener { onRemove(d.id, d.title) }
            },
        )
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
