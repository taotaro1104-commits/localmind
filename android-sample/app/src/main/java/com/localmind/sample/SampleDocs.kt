package com.localmind.sample

import android.content.Context
import java.io.File

/** 初回に登録する資料（サンプル）。Markdown なので、見出しが出典に出る。 */
data class SampleDoc(val id: String, val title: String, val markdown: String)

private val SAMPLE_DOCS_JA = listOf(
    SampleDoc(
        "contract",
        "業務委託契約書",
        """
# 業務委託契約書

## 第12条（解除）

契約書の解除は第12条に定める。解除するには30日前までに書面で通知する。

## 第15条（秘密保持）

受託者は、業務で知り得た委託者の情報を、契約の終了後も第三者に開示してはならない。
        """.trimIndent(),
    ),
    SampleDoc(
        "rules",
        "就業規則",
        """
# 就業規則

## 休暇

年次有給休暇は入社6か月後に10日付与され、取得は1時間単位でも申請できる。

## 経費

出張の経費精算は帰着後5営業日以内に申請する。
取引先との会食は1人あたり1万円を上限とし、事前に部門長の承認を得る。

## 在宅勤務

在宅勤務は前日の17時までに上長へ申請する。緊急の場合は当日の朝9時までに連絡する。
        """.trimIndent(),
    ),
    SampleDoc(
        "minutes",
        "議事録の取り扱い規程",
        """
# 議事録の取り扱い規程

## 保存と削除

議事録の保存期間は30日とし、期限を過ぎた録音は自動的に削除される。

## 録音の同意

録音を始める前に、参加者全員の同意を確認する。同意のない録音は保存しない。
        """.trimIndent(),
    ),
)

private val SAMPLE_DOCS_EN = listOf(
    SampleDoc(
        "contract",
        "Service Agreement",
        """
# Service Agreement

## Article 12 (Termination)

Termination is governed by Article 12. To terminate, give written notice at least 30 days in advance.

## Article 15 (Confidentiality)

The contractor must not disclose the client's information learned in the course of the work to any third party, even after the agreement ends.
        """.trimIndent(),
    ),
    SampleDoc(
        "rules",
        "Employment Rules",
        """
# Employment Rules

## Leave

Annual paid leave of 10 days is granted six months after joining, and it can be requested in one-hour units.

## Expenses

Submit business-trip expense reports within 5 business days of returning.
Business meals with clients are capped at 10,000 yen per person, with prior approval from the department head.

## Remote work

Request remote work from your manager by 5 p.m. the day before. In an emergency, contact them by 9 a.m. on the day.
        """.trimIndent(),
    ),
    SampleDoc(
        "minutes",
        "Minutes Handling Policy",
        """
# Minutes Handling Policy

## Retention and deletion

Minutes are kept for 30 days, and recordings past that date are deleted automatically.

## Consent to record

Before recording starts, confirm the consent of all participants. A recording without consent is not saved.
        """.trimIndent(),
    ),
)

/** 画面に出す質問の例。1 つ目は言い換え、2 つ目は別の資料、3 つ目は資料に無い話題。 */
private val SAMPLE_QUESTIONS_JA = listOf(
    "契約をやめたいときはどうすればいいですか",
    "録音データはいつ消えますか",
    "宇宙旅行の費用はいくらですか",
)

private val SAMPLE_QUESTIONS_EN = listOf(
    "How do I cancel the contract?",
    "When are recordings deleted?",
    "How much does a trip to space cost?",
)

fun sampleDocs(lang: Lang): List<SampleDoc> = if (lang == Lang.EN) SAMPLE_DOCS_EN else SAMPLE_DOCS_JA

fun sampleQuestions(lang: Lang): List<String> = if (lang == Lang.EN) SAMPLE_QUESTIONS_EN else SAMPLE_QUESTIONS_JA

private const val SEEDED_MARKER = "workspace-seeded"

/**
 * サンプルの資料の言語。登録済みなら、そのとき登録した言語（資料と例の質問の言語をそろえる）。
 * 前の版の記録（`1`）は日本語。まだなら、いまの画面の言語。
 */
fun sampleLang(context: Context): Lang {
    val marker = File(context.filesDir, SEEDED_MARKER)
    if (!marker.isFile) return I18n.lang
    return if (runCatching { marker.readText().trim() }.getOrNull() == "en") Lang.EN else Lang.JA
}

/** サンプルを登録した印（言語つき）。 */
fun markSeeded(context: Context, lang: Lang) {
    File(context.filesDir, SEEDED_MARKER).writeText(lang.name.lowercase())
}

fun isSeeded(context: Context): Boolean = File(context.filesDir, SEEDED_MARKER).exists()
