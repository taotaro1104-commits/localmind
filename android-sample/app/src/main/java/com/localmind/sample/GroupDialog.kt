package com.localmind.sample

import android.app.AlertDialog
import android.content.Context
import android.text.InputFilter
import android.widget.EditText

/** グループ名の最大文字数（Rust 側の上限と同じ）。 */
internal const val MAX_GROUP_CHARS = 40

/**
 * 資料のグループを選ぶ。既存のグループ、「未分類にする」、「新しいグループ…」から選ぶ。
 * 選んだら `onChosen`（空文字は未分類）を呼ぶ。名前の検査は Rust 側が最終的に行う。
 */
internal fun pickGroup(context: Context, current: String, existing: List<String>, onChosen: (String) -> Unit) {
    val choices = existing.filter { it != current }
    val labels = choices.toMutableList()
    if (current.isNotEmpty()) labels += "未分類にする"
    labels += "新しいグループ…"
    AlertDialog.Builder(context)
        .setTitle("グループを選ぶ" + if (current.isEmpty()) "" else "（いま: $current）")
        .setItems(labels.toTypedArray()) { _, which ->
            when {
                which < choices.size -> onChosen(choices[which])
                labels[which] == "未分類にする" -> onChosen("")
                else -> askNewGroup(context, onChosen)
            }
        }
        .setNegativeButton("キャンセル", null)
        .show()
}

private fun askNewGroup(context: Context, onChosen: (String) -> Unit) {
    val input = EditText(context).apply {
        hint = "グループ名（${MAX_GROUP_CHARS} 文字まで）"
        isSingleLine = true
        filters = arrayOf(InputFilter.LengthFilter(MAX_GROUP_CHARS))
        contentDescription = "新しいグループの名前"
    }
    AlertDialog.Builder(context)
        .setTitle("新しいグループ")
        .setView(input)
        .setPositiveButton("決める") { _, _ ->
            val name = input.text.toString().trim()
            if (name.isNotEmpty()) onChosen(name)
        }
        .setNegativeButton("キャンセル", null)
        .show()
}
