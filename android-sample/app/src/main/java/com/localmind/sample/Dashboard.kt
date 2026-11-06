package com.localmind.sample

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

private enum class Overall { RUNNING, PASS, FAIL }

/** `(123ms)` のような末尾の所要時間を、名前から切り離す。 */
private val DURATION = Regex("""\s*\((\d+)ms\)$""")

private fun formatDuration(ms: Long): String =
    if (ms >= 1000) String.format(t("%.1f 秒", "%.1f s"), ms / 1000.0) else "$ms ms"

/**
 * 自己テストの結果画面。上に全体の状態（色の帯と、1 項目 1 目盛りのテープ）、下に段階ごとの一覧を置く。
 * 画面の更新はすべて UI スレッドから呼ぶこと。
 */
class Dashboard(private val context: Context) {
    private val dp = context.resources.displayMetrics.density
    private fun px(v: Int) = (v * dp).toInt()

    private val headline = label(30f, Palette.white, bold = true)
    private val summary = label(15f, Palette.white)
    private val tape = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
    private val banner = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(headline)
        addView(summary, lp(top = 4))
        addView(tape, lp(top = 20, height = 10))
    }
    private val list = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(px(20), px(8), px(20), px(32))
    }
    private val column = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(banner)
        addView(list)
    }
    private val scroll = ScrollView(context).apply {
        setBackgroundColor(Palette.paper)
        addView(column)
        // API 35 では画面が端まで広がる。バーの下に隠れないよう、上下の余白は帯と一覧が受け持つ。
        setOnApplyWindowInsetsListener { _, insets ->
            @Suppress("DEPRECATION")
            banner.setPadding(px(20), insets.systemWindowInsetTop + px(28), px(20), px(24))
            @Suppress("DEPRECATION")
            list.setPadding(px(20), px(8), px(20), insets.systemWindowInsetBottom + px(32))
            insets
        }
    }

    val view: View get() = scroll

    private var passed = 0
    private var failed = 0
    private var running = true
    private var currentStage: LinearLayout? = null
    private var spinnerRow: View? = null

    /** 終わった段階。合格した段階は、次の段階が始まったら畳む（見出しを押すと開く）。 */
    private class Finished(val rows: View, val header: TextView, val passed: Boolean)
    private val finished = mutableListOf<Finished>()

    init {
        render()
    }

    /** 段階の見出しと、実行中の表示を出す。 */
    fun begin(title: String) {
        finished.filter { it.passed }.forEach { setOpen(it, false) }
        val section = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        section.addView(sectionHeader(title, null), lp(top = 28))
        val row = LinearLayout(context).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, px(14), 0, px(14))
            addView(mark("…", Palette.running), LinearLayout.LayoutParams(px(22), px(22)))
            addView(label(15f, Palette.running).apply { text = t("実行中…（実モデルは読み込みに時間がかかります）", "Running… (loading the real model takes a while)") }, lp(left = 14))
        }
        section.addView(row)
        spinnerRow = row
        currentStage = section
        list.addView(section)
    }

    /** 段階の結果を一覧に置き換える。 */
    fun finish(title: String, results: List<Check>) {
        val section = currentStage ?: return
        section.removeAllViews()
        val ok = results.count { it.ok }
        val header = TextView(context)
        val rows = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        results.forEachIndexed { i, check -> rows.addView(row(check, last = i == results.lastIndex)) }
        val entry = Finished(rows, header, passed = ok == results.size)
        section.addView(sectionHeader(title, "$ok / ${results.size}", header, entry), lp(top = 28))
        section.addView(rows)
        finished += entry
        setOpen(entry, true)
        results.forEach { addTick(it.ok) }
        // 段階の境目に間隔を空ける。
        (tape.getChildAt(tape.childCount - 1)?.layoutParams as? LinearLayout.LayoutParams)?.marginEnd = px(12)
        passed += ok
        failed += results.size - ok
        spinnerRow = null
        currentStage = null
        render()
    }

    private fun setOpen(entry: Finished, open: Boolean) {
        entry.rows.visibility = if (open) View.VISIBLE else View.GONE
        entry.header.text = if (open) "▾" else "▸"
    }

    /** 結果ではなく、案内だけを出す（自己テストを再実行できないとき）。 */
    fun notice(message: String) {
        running = false
        headline.text = t("実行済みです", "Already run")
        summary.text = message
    }

    /** すべての段階が終わった。合格していれば、検索画面へ進むボタンを出す。 */
    fun done(onOpenSearch: () -> Unit, onOpenRecord: () -> Unit, onOpenAsk: () -> Unit) {
        running = false
        render()
        if (failed == 0) {
            val open = label(16f, Palette.pass, bold = true).apply {
                text = t("資料を検索する", "Search documents")
                gravity = Gravity.CENTER
                setPadding(px(22), px(13), px(22), px(13))
                background = GradientDrawable().apply { setColor(Palette.white) }
                isClickable = true
                contentDescription = t("検索画面を開く", "Open the search screen")
                setOnClickListener { onOpenSearch() }
            }
            val record = label(16f, Palette.pass, bold = true).apply {
                text = t("録音する", "Record")
                gravity = Gravity.CENTER
                setPadding(px(22), px(13), px(22), px(13))
                background = GradientDrawable().apply { setColor(Palette.white) }
                isClickable = true
                contentDescription = t("録音画面を開く", "Open the recording screen")
                setOnClickListener { onOpenRecord() }
            }
            val ask = label(16f, Palette.pass, bold = true).apply {
                text = t("AI に質問する", "Ask the AI")
                gravity = Gravity.CENTER
                setPadding(px(22), px(13), px(22), px(13))
                background = GradientDrawable().apply { setColor(Palette.white) }
                isClickable = true
                contentDescription = t("質問画面を開く", "Open the question screen")
                setOnClickListener { onOpenAsk() }
            }
            val buttons = LinearLayout(context).apply {
                addView(open, LinearLayout.LayoutParams(-2, -2))
                addView(record, LinearLayout.LayoutParams(-2, -2).apply { marginStart = px(12) })
            }
            banner.addView(buttons, LinearLayout.LayoutParams(-2, -2).apply { topMargin = px(20) })
            banner.addView(ask, LinearLayout.LayoutParams(-2, -2).apply { topMargin = px(12) })
        }
    }

    /** 実行中のあいだも、目盛りと件数を更新する。 */
    private fun render() {
        val state = when {
            failed > 0 -> Overall.FAIL
            running -> Overall.RUNNING
            else -> Overall.PASS
        }
        val (color, text) = when (state) {
            Overall.RUNNING -> Palette.running to t("確認しています", "Checking")
            Overall.PASS -> Palette.pass to t("すべて合格", "All checks passed")
            Overall.FAIL -> Palette.fail to t("失敗があります", "Some checks failed")
        }
        banner.setBackgroundColor(color)
        headline.text = text
        summary.text = when {
            failed > 0 -> t("$failed 項目が失敗、$passed 項目が合格", "$failed failed, $passed passed")
            passed == 0 -> t("端末の中だけで検索を試しています", "Trying search on the device only")
            else -> t("$passed 項目が合格", "$passed passed")
        }
    }

    private fun addTick(ok: Boolean) {
        val tick = View(context).apply {
            // 合格は白、失敗は帯の上でも目立つよう暗い色にする。
            setBackgroundColor(if (ok) Color.argb(235, 255, 255, 255) else Palette.ink)
        }
        tape.addView(tick, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f).apply {
            marginEnd = px(3)
        })
        render()
    }

    private fun sectionHeader(
        title: String,
        count: String?,
        toggle: TextView? = null,
        entry: Finished? = null,
    ) = LinearLayout(context).apply {
        gravity = Gravity.BOTTOM
        setPadding(0, 0, 0, px(8))
        addView(label(18f, Palette.ink, bold = true).apply { text = title }, LinearLayout.LayoutParams(0, -2, 1f))
        if (count != null) addView(label(15f, Palette.muted, tabular = true).apply { text = count })
        if (toggle != null && entry != null) {
            toggle.apply {
                textSize = 15f
                setTextColor(Palette.muted)
                gravity = Gravity.CENTER
            }
            addView(toggle, LinearLayout.LayoutParams(px(28), -2))
            // 合格した段階だけ畳める。失敗がある段階は、いつも開いたままにする。
            if (entry.passed) {
                isClickable = true
                contentDescription = t("$title の結果を開閉する", "Show or hide the results of $title")
                setOnClickListener { setOpen(entry, entry.rows.visibility != View.VISIBLE) }
            } else {
                toggle.visibility = View.GONE
            }
        }
        background = bottomLine(Palette.ink, px(2))
    }

    private fun row(check: Check, last: Boolean): View {
        val match = DURATION.find(check.name)
        val name = match?.let { check.name.removeRange(it.range) } ?: check.name
        val duration = match?.groupValues?.get(1)?.toLongOrNull()?.let(::formatDuration)
        val color = if (check.ok) Palette.pass else Palette.fail

        val text = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            addView(label(15f, Palette.ink).apply { this.text = name })
            if (!check.ok && check.detail.isNotEmpty()) {
                addView(label(13f, Palette.fail).apply { this.text = check.detail }, lp(top = 4))
            }
        }
        return LinearLayout(context).apply {
            gravity = Gravity.TOP
            setPadding(0, px(13), 0, px(13))
            if (!last) background = bottomLine(Palette.line, px(1))
            addView(mark(check.ok, color), LinearLayout.LayoutParams(px(22), px(22)).apply { topMargin = px(1) })
            addView(text, LinearLayout.LayoutParams(0, -2, 1f).apply { marginStart = px(14); marginEnd = px(12) })
            if (duration != null) addView(label(13f, Palette.muted, tabular = true).apply { this.text = duration })
        }
    }

    private fun mark(ok: Boolean, color: Int) = mark(if (ok) "✓" else "×", color)

    private fun mark(glyph: String, color: Int) = TextView(context).apply {
        text = glyph
        textSize = 13f
        setTextColor(Palette.white)
        setTypeface(typeface, Typeface.BOLD)
        gravity = Gravity.CENTER
        background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(color)
        }
    }

    private fun bottomLine(color: Int, thickness: Int) = android.graphics.drawable.LayerDrawable(
        arrayOf(
            GradientDrawable().apply { setColor(color) },
            GradientDrawable().apply { setColor(Palette.paper) },
        ),
    ).apply { setLayerInset(1, 0, 0, 0, thickness) }

    private fun label(size: Float, color: Int, bold: Boolean = false, tabular: Boolean = false) = TextView(context).apply {
        textSize = size
        setTextColor(color)
        if (bold) setTypeface(Typeface.create("sans-serif-medium", Typeface.BOLD))
        if (tabular) fontFeatureSettings = "tnum"
        setLineSpacing(0f, 1.15f)
        setTextIsSelectable(true)
    }

    private fun lp(top: Int = 0, left: Int = 0, height: Int = ViewGroup.LayoutParams.WRAP_CONTENT) =
        LinearLayout.LayoutParams(
            if (left == 0) ViewGroup.LayoutParams.MATCH_PARENT else ViewGroup.LayoutParams.WRAP_CONTENT,
            if (height > 0) px(height) else height,
        ).apply {
            topMargin = px(top)
            marginStart = px(left)
        }
}
