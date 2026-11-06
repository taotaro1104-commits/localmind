package com.localmind.sample

import java.util.Locale

/** 画面の言語。端末の言語が日本語なら日本語、それ以外は英語。 */
enum class Lang { JA, EN }

/**
 * 画面の言語の判定。端末（Android 13 以降は、アプリごとの言語設定）の言語に従う。
 * テストでは [forced] で固定する。
 */
internal object I18n {
    @Volatile var forced: Lang? = null

    val lang: Lang get() = forced ?: if (Locale.getDefault().language == "ja") Lang.JA else Lang.EN
}

/**
 * 日本語と英語の文言を並べて書き、いまの言語のほうを返す。文言は使う場所に置き、`$名前` で値を差し込む
 * （Context を持ち回らずに、どこからでも呼べて、JVM のテストにも使える）。
 * 値は両方の文言で評価されるので、副作用のある式は渡さない。
 */
internal fun t(ja: String, en: String): String = if (I18n.lang == Lang.JA) ja else en
