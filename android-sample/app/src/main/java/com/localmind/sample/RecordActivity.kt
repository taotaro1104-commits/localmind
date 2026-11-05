package com.localmind.sample

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowInsetsController
import kotlin.concurrent.thread
import uniffi.lm_ffi.RecordingInfo

private const val REQUEST_PERMISSIONS = 2001
private const val PURGE_JOB_ID = 7001
private const val ONE_DAY_MS = 24L * 60 * 60 * 1000

/**
 * 録音の画面。録音の前に参加者の同意を確認し（設定でオフにできる）、録音中は止められる通知を常に出す。
 * 保存期間を過ぎた録音は、画面を開いたとき・録音を止めたとき・1 日ごとに自動で消える。
 */
class RecordActivity : Activity() {
    private lateinit var screen: RecordScreen
    private lateinit var prefs: RecordPrefs
    private val ticker = Handler(Looper.getMainLooper())
    private val debuggable by lazy { applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0 }
    private var wasActive = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.hide()
        prefs = RecordPrefs(this)
        screen = RecordScreen(this, ::toggle, ::setConfirmConsent, ::setRetention, ::confirmDelete)
        setContentView(screen.view)
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            window.insetsController?.setSystemBarsAppearance(0, WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS)
        }
        screen.setSettings(prefs.confirmConsent, prefs.retentionDays)
        scheduleDailyPurge()
        thread(name = "lm-purge") {
            val failure = Purger.run(applicationContext)
            runOnUiThread { failure?.let { screen.note(it, ok = false) }; refresh() }
        }
    }

    override fun onResume() {
        super.onResume()
        tick()
    }

    override fun onPause() {
        ticker.removeCallbacksAndMessages(null)
        super.onPause()
    }

    /** 1 秒ごとに、録音の経過時間と状態を更新する。録音が終わったら、一覧を読み直す。 */
    private fun tick() {
        val state = RecorderState.snapshot
        screen.setActive(state.active, System.currentTimeMillis() - state.startedAtMs, state.savedChunks)
        val error = RecorderState.takeError()
        if (error != null) screen.note(error, ok = false)
        if (wasActive && !state.active) {
            refresh()
            if (error == null) screen.note("録音を保存しました", ok = true)
        }
        wasActive = state.active
        ticker.postDelayed(::tick, 1000)
    }

    private fun toggle() {
        if (RecorderState.active) {
            RecorderService.stop(this)
            screen.note("止めています…（最後の分を保存しています）", ok = true)
        } else {
            requestPermissionsThenStart()
        }
    }

    /** 録音に要る許可（マイクと、録音中の通知）をそろえてから始める。 */
    private fun requestPermissionsThenStart() {
        val needed = buildList {
            add(Manifest.permission.RECORD_AUDIO)
            if (android.os.Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (needed.isEmpty()) confirmConsentThenStart() else requestPermissions(needed.toTypedArray(), REQUEST_PERMISSIONS)
    }

    @Deprecated("Activity の素の API。AndroidX を使わないサンプルなので、これを使う")
    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQUEST_PERMISSIONS) return
        val denied = permissions.filterIndexed { i, _ -> grantResults.getOrNull(i) != PackageManager.PERMISSION_GRANTED }
        when {
            denied.isEmpty() -> confirmConsentThenStart()
            Manifest.permission.RECORD_AUDIO in denied -> screen.note("マイクの許可がないと、録音できません", ok = false)
            else -> screen.note("録音中の通知を許可してください（録音していることを、いつでも分かるようにするためです）", ok = false)
        }
    }

    /** 同意の確認（設定でオフにしていれば、確認せずに始める）。同意が確認されないと、録音は始まらない。 */
    private fun confirmConsentThenStart() {
        if (!prefs.confirmConsent) {
            RecorderService.start(this, consentConfirmed = true)
            return
        }
        AlertDialog.Builder(this)
            .setTitle("録音を始める前の確認")
            .setMessage("録音に参加する全員から、録音することの同意を得ていますか？\n\n同意を得ていない録音は、始められません。")
            .setPositiveButton("同意を得た。録音を始める") { _, _ -> RecorderService.start(this, consentConfirmed = true) }
            .setNegativeButton("キャンセル", null)
            .show()
    }

    /** 同意の確認をオフにするときは、念のためもう一度確かめる。 */
    private fun setConfirmConsent(on: Boolean) {
        if (on) {
            prefs.confirmConsent = true
            return
        }
        AlertDialog.Builder(this)
            .setTitle("同意の確認をやめますか？")
            .setMessage("録音を始めるたびの確認がなくなります。参加者の同意を得る責任は、あなたにあります。")
            .setPositiveButton("確認をやめる") { _, _ -> prefs.confirmConsent = false }
            .setNegativeButton("キャンセル") { _, _ -> screen.setSettings(prefs.confirmConsent, prefs.retentionDays) }
            .setOnCancelListener { screen.setSettings(prefs.confirmConsent, prefs.retentionDays) }
            .show()
    }

    private fun setRetention(days: Int) {
        prefs.retentionDays = days
        screen.setSettings(prefs.confirmConsent, days)
        thread(name = "lm-purge") {
            val failure = Purger.run(applicationContext)
            runOnUiThread { failure?.let { screen.note(it, ok = false) }; refresh() }
        }
    }

    private fun confirmDelete(r: RecordingInfo) {
        if (RecorderState.active) {
            screen.note("録音中は削除できません。止めてからお試しください", ok = false)
            return
        }
        AlertDialog.Builder(this)
            .setTitle("この録音を削除しますか？")
            .setMessage("音声が端末から消えます。元に戻せません。")
            .setPositiveButton("削除") { _, _ -> delete(r.id) }
            .setNegativeButton("キャンセル", null)
            .show()
    }

    private fun delete(id: String) {
        thread(name = "lm-delete") {
            val result = runCatching { openRecordingStore(applicationContext).deleteRecording(id) }
            runOnUiThread {
                result.onSuccess { screen.note("削除しました", ok = true) }
                    .onFailure { screen.note(userMessage(it, debuggable, "削除できませんでした"), ok = false) }
                refresh()
            }
        }
    }

    /** 保存した録音の一覧を読み直す。 */
    private fun refresh() {
        thread(name = "lm-list") {
            val result = runCatching { openRecordingStore(applicationContext).recordings() }
            runOnUiThread {
                if (isDestroyed) return@runOnUiThread
                result.onSuccess { screen.setRecordings(it, deletable = !RecorderState.active) }
                    .onFailure { screen.note(userMessage(it, debuggable, "録音の一覧を読めませんでした"), ok = false) }
            }
        }
    }

    /** 1 日ごとに、保存期間を過ぎた録音を消す（すでに予約済みなら何もしない）。 */
    private fun scheduleDailyPurge() {
        val scheduler = getSystemService(JobScheduler::class.java)
        if (scheduler.getPendingJob(PURGE_JOB_ID) != null) return
        scheduler.schedule(
            JobInfo.Builder(PURGE_JOB_ID, ComponentName(this, PurgeJob::class.java))
                .setPeriodic(ONE_DAY_MS)
                .setPersisted(true) // 再起動後も続ける（RECEIVE_BOOT_COMPLETED が要る）
                .build(),
        )
    }
}
