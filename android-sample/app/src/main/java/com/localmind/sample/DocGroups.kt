package com.localmind.sample

import uniffi.lm_ffi.DocumentInfo

/** 一覧の 1 区画。`key` が空なら未分類。 */
internal data class DocSection(val key: String, val docs: List<DocumentInfo>) {
    val label: String get() = if (key.isEmpty()) UNGROUPED_LABEL else key
}

internal val UNGROUPED_LABEL: String get() = t("未分類", "Ungrouped")

/** 資料の一覧をグループ（資料に付けた名前）ごとにまとめる。表示の組み立てだけで、状態は持たない。 */
internal object DocGroups {
    /** グループ名の順（五十音・英字の並び）に並べ、未分類は最後に置く。資料の並びは入力のまま。 */
    fun sections(docs: List<DocumentInfo>): List<DocSection> {
        val byGroup = docs.groupBy { it.group }
        val named = byGroup.keys.filter { it.isNotEmpty() }.sortedWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it })
        return (named + listOfNotNull(if (byGroup.containsKey("")) "" else null)).map { DocSection(it, byGroup.getValue(it)) }
    }

    /** 名前の付いたグループだけ（選択肢に出す）。 */
    fun names(docs: List<DocumentInfo>): List<String> = sections(docs).map { it.key }.filter { it.isNotEmpty() }

    /** 選んだグループの区画だけ。`selected` が null なら、すべて。 */
    fun visible(sections: List<DocSection>, selected: String?): List<DocSection> =
        if (selected == null) sections else sections.filter { it.key == selected }

    /** 選んでいたグループが無くなっていたら、「すべて」（null）に戻す。 */
    fun keepSelection(sections: List<DocSection>, selected: String?): String? =
        selected?.takeIf { s -> sections.any { it.key == s } }
}
