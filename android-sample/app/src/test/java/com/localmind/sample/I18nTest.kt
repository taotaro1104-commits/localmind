package com.localmind.sample

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class I18nTest {
    private val japanese = Regex("[぀-ヿ一-鿿＀-￯]")

    @After fun reset() {
        I18n.forced = null
    }

    @Test
    fun theTextFollowsTheChosenLanguage() {
        I18n.forced = Lang.JA
        assertEquals("日本語", t("日本語", "English"))
        I18n.forced = Lang.EN
        assertEquals("English", t("日本語", "English"))
        // 値は両方の文言に差し込める
        val n = 3
        assertEquals("3 items", t("$n 件", "$n items"))
    }

    @Test
    fun theDeviceLanguageDecidesWhenNothingIsForced() {
        val saved = java.util.Locale.getDefault()
        try {
            java.util.Locale.setDefault(java.util.Locale.JAPAN)
            assertEquals(Lang.JA, I18n.lang)
            for (l in listOf(java.util.Locale.US, java.util.Locale.FRANCE, java.util.Locale.KOREA)) {
                java.util.Locale.setDefault(l)
                assertEquals(l.toString(), Lang.EN, I18n.lang)
            }
        } finally {
            java.util.Locale.setDefault(saved)
        }
    }

    @Test
    fun modelNamesAreTranslated() {
        I18n.forced = Lang.JA
        val ja = MODEL_CATALOG.map { it.label }
        I18n.forced = Lang.EN
        val en = MODEL_CATALOG.map { it.label }
        assertTrue(ja.all { japanese.containsMatchIn(it) })
        assertTrue("英語の名前に日本語が残っている: $en", en.none { japanese.containsMatchIn(it) })
        assertNotEquals(ja, en)
    }

    @Test
    fun sampleDocumentsAndQuestionsExistInBothLanguages() {
        for (lang in Lang.values()) {
            val docs = sampleDocs(lang)
            assertEquals(listOf("contract", "rules", "minutes"), docs.map { it.id })
            assertEquals(3, sampleQuestions(lang).size)
            val hasJapanese = docs.any { japanese.containsMatchIn(it.title + it.markdown) }
            assertEquals(lang.name, lang == Lang.JA, hasJapanese)
        }
    }

    @Test
    fun rustErrorsAreTranslatedInEnglishAndLeftAloneInJapanese() {
        val cases = mapOf(
            "文書が大きすぎます（300000 バイト。上限は 262144 バイト）" to "The document is too large (300000 bytes; the limit is 262144 bytes)",
            "登録できる資料がいっぱいです（200 件、または合計 16777216 バイトまで）。不要な資料を削除してください" to
                "The document store is full (up to 200 documents or 16777216 bytes in total). Please delete documents you do not need",
            "題名が不正です: 空です" to "The title is invalid: it is empty",
            "題名が不正です: 長すぎます" to "The title is invalid: it is too long",
            "グループ名が長すぎます" to "The group name is too long",
            "入出力エラー: 権限がありません" to "I/O error: Permission denied",
            "質問が長すぎます（500 文字まで）" to "The question is too long (up to 500 characters)",
            "LLM の出力を読めません: text がありません" to "Cannot read the LLM output: text is missing",
            "モデルファイルを使えません: 内容が固定したハッシュと一致しません（破損か差し替え）" to
                "Cannot use the model file: the contents do not match the pinned hash (damaged or replaced)",
            "モデルのパスは `..` を含まない絶対パスで指定してください" to "The model path must be an absolute path without '..'",
            "文字起こしが長すぎます。短い区間に分けて抽出してください" to "The transcript is too long. Please split it into shorter sections and extract them separately",
        )
        for ((ja, en) in cases) {
            I18n.forced = Lang.EN
            assertEquals(ja, en, ErrorText.localize(ja))
            I18n.forced = Lang.JA
            assertEquals("日本語はそのまま", ja, ErrorText.localize(ja))
        }
    }

    @Test
    fun anUntranslatableMessageIsNeverShownInEnglish() {
        I18n.forced = Lang.EN
        assertNull(ErrorText.localize("まだ訳していない新しいエラー"))
        assertNull(ErrorText.localize("題名が不正です: 知らない理由"))
        assertEquals("plain English stays", ErrorText.localize("plain English stays"))
        assertNull(ErrorText.localize(null))
    }

    @Test
    fun englishTextHasNoJapaneseLeftOverInTheModelCatalogOrErrors() {
        I18n.forced = Lang.EN
        val e = ErrorText.localize("不正な録音 id: x")
        assertFalse(japanese.containsMatchIn(e.orEmpty()))
    }
}
