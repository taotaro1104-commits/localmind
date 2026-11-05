package com.localmind.sample

import android.content.Context
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.view.Gravity
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView

/** 画面の部品を作る小さな道具（色は [Palette]）。 */
internal class Ui(val context: Context) {
    private val dp = context.resources.displayMetrics.density
    fun px(v: Int) = (v * dp).toInt()

    fun text(size: Float, color: Int, bold: Boolean = false) = TextView(context).apply {
        textSize = size
        setTextColor(color)
        if (bold) setTypeface(Typeface.create("sans-serif-medium", Typeface.BOLD))
        setLineSpacing(0f, 1.2f)
    }

    /** 下に線を引く背景（紙の色の上に、`thickness` の太さで `color`）。 */
    fun underline(color: Int, thickness: Int) = LayerDrawable(
        arrayOf(GradientDrawable().apply { setColor(color) }, GradientDrawable().apply { setColor(Palette.paper) }),
    ).apply { setLayerInset(1, 0, 0, 0, thickness) }

    fun fill(color: Int) = GradientDrawable().apply { setColor(color) }

    /** 塗りつぶしのボタン。押せるのは [TextView.isEnabled] が true のときだけ。 */
    fun button(label: String, color: Int, description: String = label, onClick: () -> Unit) =
        text(16f, Palette.white, bold = true).apply {
            text = label
            gravity = Gravity.CENTER
            setPadding(px(22), px(13), px(22), px(13))
            background = fill(color)
            contentDescription = description
            isClickable = true
            setOnClickListener { if (isEnabled) onClick() }
        }

    fun lp(top: Int = 0, left: Int = 0, wrap: Boolean = false) = LinearLayout.LayoutParams(
        if (wrap) ViewGroup.LayoutParams.WRAP_CONTENT else ViewGroup.LayoutParams.MATCH_PARENT,
        ViewGroup.LayoutParams.WRAP_CONTENT,
    ).apply {
        topMargin = px(top)
        marginStart = px(left)
    }
}
