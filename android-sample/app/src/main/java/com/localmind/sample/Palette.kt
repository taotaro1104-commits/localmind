package com.localmind.sample

import android.graphics.Color

/** 配色。結果の状態（合格・実行中・失敗）を色で一目で分けるため、状態の色はこの 3 色に限る。 */
internal object Palette {
    val paper = Color.parseColor("#F2F5F4")
    val ink = Color.parseColor("#101C26")
    val muted = Color.parseColor("#5B6B73")
    val line = Color.parseColor("#D3DBD9")
    val pass = Color.parseColor("#0E7A5F")
    val running = Color.parseColor("#9A5B00")
    val fail = Color.parseColor("#B3261E")
    val white = Color.WHITE
}
