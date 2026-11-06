package com.localmind.sample

import org.junit.After
import org.junit.Before

/** 日本語の文言を確かめるテストの土台。実行する端末の言語に左右されないよう、画面の言語を日本語に固定する。 */
abstract class JapaneseUi {
    @Before fun forceJapanese() {
        I18n.forced = Lang.JA
    }

    @After fun resetLanguage() {
        I18n.forced = null
    }
}
