package com.localmind.sample

import android.content.Context
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import uniffi.lm_ffi.RecordingInfo

/** 録音画面の見た目。状態の変更はすべて UI スレッドから呼ぶこと。 */
class RecordScreen(
    context: Context,
    private val onToggle: () -> Unit,
    private val onConsentPref: (Boolean) -> Unit,
    private val onRetention: (Int) -> Unit,
    private val onMeetingLanguage: (String) -> Unit,
    private val onDelete: (RecordingInfo) -> Unit,
    private val onTranscribe: (RecordingInfo) -> Unit,
) {
    private val ui = Ui(context)
    private val px = ui::px

    private val title = ui.text(28f, Palette.white, bold = true).apply { text = t("録音", "Recording") }
    private val status = ui.text(15f, Palette.white)
    private val timer = ui.text(40f, Palette.white, bold = true).apply { fontFeatureSettings = "tnum" }
    private val toggle = ui.text(17f, Palette.ink, bold = true).apply {
        gravity = Gravity.CENTER
        setPadding(px(24), px(14), px(24), px(14))
        background = ui.fill(Palette.white)
        isClickable = true
        setOnClickListener { if (isEnabled) onToggle() }
    }
    private val banner = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(title)
        addView(status, ui.lp(top = 4))
        addView(timer, ui.lp(top = 12))
        addView(toggle, ui.lp(top = 20, wrap = true))
    }

    private val consentSwitch = Switch(context).apply {
        text = t("録音の前に、参加者の同意を確認する", "Confirm consent before recording")
        textSize = 16f
        setTextColor(Palette.ink)
        setPadding(0, px(14), 0, px(14))
    }
    private val retention = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
    private val meetingLanguage = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
    private val list = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val listHeading = heading(t("保存した録音", "Saved recordings"))
    private val message = ui.text(14f, Palette.muted)

    private val body = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        addView(heading(t("設定", "Settings")), ui.lp(top = 4))
        addView(consentSwitch)
        addView(ui.text(15f, Palette.ink, bold = true).apply { text = t("録音の保存期間", "How long to keep recordings") }, ui.lp(top = 12))
        addView(retention, ui.lp(top = 8))
        addView(ui.text(13f, Palette.muted).apply { text = t("期間を過ぎた録音は、自動で削除されます。", "Recordings older than this are deleted automatically.") }, ui.lp(top = 8))
        addView(ui.text(15f, Palette.ink, bold = true).apply { text = t("会議の言語（文字起こしと議事録）", "Language of the meeting (transcription and minutes)") }, ui.lp(top = 16))
        addView(meetingLanguage, ui.lp(top = 8))
        addView(
            ui.text(13f, Palette.muted).apply {
                text = t("会議で話される言葉を選びます。文字起こしと、議事録を書く言語の、両方に使います。", "Choose the language spoken in the meeting. It is used for both the transcription and the language the minutes are written in.")
            },
            ui.lp(top = 8),
        )
        addView(listHeading, ui.lp(top = 32))
        addView(list)
        addView(message, ui.lp(top = 12))
    }
    private val scroll = ScrollView(context).apply {
        setBackgroundColor(Palette.paper)
        isFillViewport = true
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            addView(banner)
            addView(body)
        })
        setOnApplyWindowInsetsListener { _, insets ->
            @Suppress("DEPRECATION")
            banner.setPadding(px(20), insets.systemWindowInsetTop + px(28), px(20), px(24))
            @Suppress("DEPRECATION")
            body.setPadding(px(20), px(20), px(20), insets.systemWindowInsetBottom + px(32))
            insets
        }
    }
    val view: View get() = scroll

    private var settingsApplying = false

    init {
        consentSwitch.setOnCheckedChangeListener { _, checked -> if (!settingsApplying) onConsentPref(checked) }
        setActive(false, 0, 0)
    }

    /** 録音中か、そうでないか。 */
    fun setActive(active: Boolean, elapsedMs: Long, chunks: Int) {
        banner.setBackgroundColor(if (active) Palette.fail else Palette.ink)
        title.text = if (active) t("録音中", "Recording") else t("録音", "Recording")
        status.text = if (active) t("端末の中だけに保存しています（保存済み $chunks 区間 ・ 30 秒ごと）", "Saved on the device only (saved $chunks parts · every 30 seconds)") else t("会議や講義を、端末の中だけに録音します", "Record meetings and lectures on the device only")
        timer.visibility = if (active) View.VISIBLE else View.GONE
        timer.text = clock(elapsedMs)
        toggle.text = if (active) t("録音を止める", "Stop recording") else t("録音を始める", "Start recording")
        toggle.contentDescription = toggle.text
    }

    fun setSettings(confirmConsent: Boolean, retentionDays: Int) {
        settingsApplying = true
        consentSwitch.isChecked = confirmConsent
        settingsApplying = false
        retention.removeAllViews()
        PrefsChoices.forEach { days ->
            val on = days == retentionDays
            retention.addView(
                ui.text(15f, if (on) Palette.white else Palette.ink, bold = on).apply {
                    text = t("$days 日", "$days days")
                    gravity = Gravity.CENTER
                    setPadding(px(18), px(10), px(18), px(10))
                    background = GradientDrawable().apply {
                        setColor(if (on) Palette.ink else Palette.white)
                        setStroke(px(1), Palette.ink)
                    }
                    isClickable = true
                    contentDescription = t("保存期間を $days 日にする", "Keep recordings for $days days")
                    setOnClickListener { onRetention(days) }
                },
                ui.lp(left = if (retention.childCount == 0) 0 else 8, wrap = true),
            )
        }
    }

    /** 会議の言語の選択肢（日本語・English）を出す。選んでいるものは濃く塗る。 */
    fun setMeetingLanguage(code: String) {
        meetingLanguage.removeAllViews()
        listOf("ja" to "日本語", "en" to "English").forEach { (c, label) ->
            val on = c == code
            meetingLanguage.addView(
                ui.text(15f, if (on) Palette.white else Palette.ink, bold = on).apply {
                    text = label
                    gravity = Gravity.CENTER
                    setPadding(px(18), px(10), px(18), px(10))
                    background = GradientDrawable().apply {
                        setColor(if (on) Palette.ink else Palette.white)
                        setStroke(px(1), Palette.ink)
                    }
                    isClickable = true
                    isSelected = on
                    contentDescription = t("会議の言語を${label}にする", "Set the meeting language to ${label}")
                    setOnClickListener { onMeetingLanguage(c) }
                },
                ui.lp(left = if (meetingLanguage.childCount == 0) 0 else 8, wrap = true),
            )
        }
    }

    fun setRecordings(recordings: List<RecordingInfo>, deletable: Boolean) {
        listHeading.text = t("保存した録音（${recordings.size} 件）", "Saved recordings (${recordings.size})")
        list.removeAllViews()
        if (recordings.isEmpty()) {
            list.addView(ui.text(14f, Palette.muted).apply {
                text = t("録音はありません。", "There are no recordings.")
                setPadding(0, px(12), 0, px(12))
            })
        }
        recordings.sortedByDescending { it.startedAtMs.toLong() }.forEachIndexed { i, r ->
            list.addView(row(r, deletable, last = i == recordings.lastIndex))
        }
    }

    fun note(text: String, ok: Boolean) {
        message.text = text
        message.setTextColor(if (ok) Palette.pass else Palette.fail)
    }

    fun enableToggle(on: Boolean) {
        toggle.isEnabled = on
        toggle.alpha = if (on) 1f else 0.5f
    }

    private fun row(r: RecordingInfo, deletable: Boolean, last: Boolean) = LinearLayout(banner.context).apply {
        gravity = Gravity.CENTER_VERTICAL
        setPadding(0, px(12), 0, px(12))
        if (!last) background = ui.underline(Palette.line, px(1))
        val what = if (r.readable) formatDate(r.startedAtMs.toLong()) else t("（情報を読めない録音）", "(recording whose details cannot be read)")
        addView(
            LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(ui.text(16f, Palette.ink, bold = true).apply { text = what })
                addView(ui.text(13f, Palette.muted).apply { text = t("${approxLength(r.chunks)} ・ ${megabytes(r.bytes)}", "${approxLength(r.chunks)} · ${megabytes(r.bytes)}") }, ui.lp(top = 2))
            },
            LinearLayout.LayoutParams(0, -2, 1f),
        )
        if (deletable) {
            addView(ui.text(14f, Palette.pass).apply {
                text = t("文字起こし", "Transcribe")
                setPadding(px(12), px(10), px(12), px(10))
                isClickable = true
                contentDescription = t("$what の録音を文字起こしする", "Transcribe the recording from $what")
                setOnClickListener { onTranscribe(r) }
            })
            addView(ui.text(14f, Palette.fail).apply {
                text = t("削除", "Delete")
                setPadding(px(12), px(10), 0, px(10))
                isClickable = true
                contentDescription = t("$what の録音を削除する", "Delete the recording from $what")
                setOnClickListener { onDelete(r) }
            })
        }
    }

    private fun heading(label: String): TextView = ui.text(18f, Palette.ink, bold = true).apply {
        text = label
        setPadding(0, 0, 0, px(8))
        background = ui.underline(Palette.ink, px(2))
    }

    private companion object {
        val PrefsChoices = RecordPrefs.CHOICES

        fun clock(ms: Long): String {
            val s = (ms / 1000).coerceAtLeast(0)
            return String.format(Locale.US, "%02d:%02d", s / 60, s % 60)
        }

        fun formatDate(ms: Long): String =
            if (I18n.lang == Lang.JA) SimpleDateFormat("M月d日 HH:mm", Locale.JAPAN).format(Date(ms))
            else SimpleDateFormat("MMM d, HH:mm", Locale.ENGLISH).format(Date(ms))

        /** チャンクは 30 秒なので、おおよその長さ。 */
        fun approxLength(chunks: UInt): String {
            val minutes = (chunks.toLong() * 30 + 59) / 60
            return if (chunks == 0u) t("音声なし", "No audio") else if (minutes <= 1) t("約 1 分以内", "about 1 min or less") else t("約 $minutes 分", "about $minutes min")
        }

        fun megabytes(bytes: ULong): String = String.format(Locale.US, "%.1f MB", bytes.toDouble() / (1024 * 1024))
    }
}
