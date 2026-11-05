package com.localmind.sample

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.IBinder
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import uniffi.lm_ffi.Recorder
import uniffi.lm_ffi.RecordingStore

private const val CHANNEL_ID = "recording"
private const val NOTIFICATION_ID = 1
private const val STOPPED_NOTIFICATION_ID = 2

/** 録音の標準のサンプルレート（音声認識に十分で、容量を抑えられる）。 */
const val RECORD_SAMPLE_RATE = 16_000

/**
 * 録音の Foreground Service（マイク）。録音中は、止められる通知を常に出す。マイクの音声は 30 秒ごとに
 * 端末内へ逐次保存する（クラッシュしても、保存済みの分までは残る）。同意が確認されていなければ始めない。
 *
 * 録音は 1 度に 1 つ。止める処理（残りの保存・古い録音の削除）が終わるまでは、新しい録音を始めない。
 * 開始は [start]、停止は [stop]。画面は [RecorderState] を読む。
 */
class RecorderService : Service() {
    /** 録音 1 回分。停止の要求は、この録音にだけ効く。 */
    private class Session {
        @Volatile var stopRequested = false
    }

    /** 今の録音。始めるのは UI スレッド、終わって null に戻すのは録音の thread なので、@Volatile。 */
    @Volatile private var session: Session? = null

    /** 録音中に出す通知（同じものを再利用する。録音中に再度 `startForeground` が要るとき、時刻を巻き戻さない）。 */
    private var ongoing: Notification? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> begin(intent.getBooleanExtra(EXTRA_CONSENT, false))
            ACTION_STOP -> session.let { if (it != null) it.stopRequested = true else stopSelf() }
            else -> if (session == null) stopSelf()
        }
        return START_NOT_STICKY // 強制終了されたら、勝手には再開しない（マイクを使うので、利用者の操作でだけ始める）
    }

    private fun begin(consent: Boolean) {
        // `startForegroundService` で呼ばれたら、すぐに `startForeground` を呼ぶ（5 秒以内。呼ばないと例外で落ちる）。
        // 実行中・停止処理中は、いまの通知のまま呼び直す。
        if (session != null) {
            ongoing?.let { foreground(it) }
            RecorderState.warn("前の録音を終えています。少し待ってから、もう一度お試しください")
            return
        }
        val startedAt = System.currentTimeMillis()
        val refusal = preconditionFailure()
        val notification = notification(startedAt)
        if (!foreground(notification) || refusal != null) {
            fail(refusal ?: "録音を始められませんでした")
            return
        }
        ongoing = notification
        val mine = Session()
        session = mine
        RecorderState.begin(startedAt)
        // マイクの準備や保存先の用意は重いので、別の thread で行う。
        Thread({ record(mine, consent, startedAt) }, "lm-record").start()
    }

    /** 録音を始める前提（マイクの許可・通知が見えること）。満たさなければ理由を返す。 */
    private fun preconditionFailure(): String? {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            return "マイクの許可がありません"
        }
        val manager = getSystemService(NotificationManager::class.java)
        channel()
        val channelOff = manager.getNotificationChannel(CHANNEL_ID)?.importance == NotificationManager.IMPORTANCE_NONE
        if (!manager.areNotificationsEnabled() || channelOff) {
            return "録音中の通知が無効です。録音していることを常に分かるようにするため、通知を許可してください"
        }
        return null
    }

    /** 前面サービスにする。失敗（許可の取り消し、起動の制限）は、落とさずに false を返す。 */
    private fun foreground(notification: Notification): Boolean =
        try {
            val type = if (android.os.Build.VERSION.SDK_INT >= 30) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
            startForeground(NOTIFICATION_ID, notification, type)
            true
        } catch (e: Exception) {
            false
        }

    /** 録音の thread。準備 → 録音 → 後片付けを、この thread で行う。 */
    private fun record(mine: Session, consent: Boolean, startedAt: Long) {
        var recorder: Recorder? = null
        var audio: AudioRecord? = null
        var failure: String? = null
        val id = newId(startedAt)
        val store = openRecordingStore(this)
        try {
            // 先にマイクを用意する（失敗したら、録音の記録を作らない）。
            audio = openMicrophone()
            recorder = store.begin(id, consent, RECORD_SAMPLE_RATE.toUInt(), startedAt.toULong())
            capture(mine, recorder, audio)
        } catch (e: Exception) {
            failure = userMessage(e, debuggable = false, fallback = "録音を続けられませんでした")
        } finally {
            failure = finish(store, id, recorder, audio, failure)
            RecorderState.end(failure)
            failure?.let { notifyStopped(it) }
            stopForeground(STOP_FOREGROUND_REMOVE)
            Purger.run(applicationContext) // 保存期間を過ぎた録音を消す（通知を消してから）
            session = null
            stopSelf()
        }
    }

    private fun newId(startedAt: Long) = "rec-" + SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date(startedAt))

    private fun openMicrophone(): AudioRecord {
        val minBuffer = AudioRecord.getMinBufferSize(RECORD_SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        check(minBuffer > 0) { "この端末では録音できません" }
        val audio = AudioRecord(
            MediaRecorder.AudioSource.MIC, RECORD_SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuffer, RECORD_SAMPLE_RATE * 2),
        )
        if (audio.state != AudioRecord.STATE_INITIALIZED) {
            audio.release()
            error("マイクを使えません（ほかのアプリが使用中かもしれません）")
        }
        return audio
    }

    /** マイクから 1 秒ずつ読んで保存する。止められるか、失敗するまで続ける。 */
    private fun capture(mine: Session, recorder: Recorder, audio: AudioRecord) {
        val buffer = ShortArray(RECORD_SAMPLE_RATE) // 1 秒ずつ
        audio.startRecording()
        while (!mine.stopRequested) {
            val n = audio.read(buffer, 0, buffer.size)
            if (n < 0) error("マイクから読めなくなりました")
            if (n > 0) RecorderState.saved(recorder.push(buffer.asList().subList(0, n).toList()).toInt())
        }
    }

    /**
     * マイクを閉じ、残りの端数を保存する。保存に失敗した録音（`failure` がある）は、中途半端なチャンクを
     * 足さない（Rust 側が拒否する）。録音を作れなかったときは、空の録音を残さない。返り値は、画面に出す失敗。
     */
    private fun finish(store: RecordingStore, id: String, recorder: Recorder?, audio: AudioRecord?, failure: String?): String? {
        audio?.let { runCatching { it.stop() }; it.release() }
        recorder ?: return failure
        val saved = runCatching { recorder.finish().toInt() }
        saved.onSuccess { RecorderState.saved(it) }
        // 音声が 1 つも保存できなかった録音（すぐに止めた・始めてすぐ失敗した）は、空の記録を残さない。
        if (recorder.savedChunks() == 0u) runCatching { store.deleteRecording(id) }
        return failure ?: saved.exceptionOrNull()?.let { userMessage(it, false, "最後の保存に失敗しました") }
    }

    /** 前提を満たさない・前面サービスにできないなどで、録音を始めずに終える。 */
    private fun fail(message: String) {
        RecorderState.end(message)
        notifyStopped(message)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun notification(startedAt: Long): Notification {
        channel()
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, RecordActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, RecorderService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle("録音中")
            .setContentText("端末の中だけに保存しています。止めるには「停止」を押してください")
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setVisibility(Notification.VISIBILITY_PUBLIC) // ロック画面にも出す（録音中であることを隠さない）
            .setWhen(startedAt)
            .setUsesChronometer(true)
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "停止", stop).build())
            .setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    private fun notifyStopped(message: String) {
        channel()
        val n = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setContentTitle("録音を止めました")
            .setContentText(message)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(STOPPED_NOTIFICATION_ID, n)
    }

    private fun channel() {
        val manager = getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL_ID) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "録音", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "録音中は、この通知を常に表示します"
                    lockscreenVisibility = Notification.VISIBILITY_PUBLIC
                },
            )
        }
    }

    override fun onDestroy() {
        session?.stopRequested = true
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "com.localmind.sample.RECORD_START"
        const val ACTION_STOP = "com.localmind.sample.RECORD_STOP"
        const val EXTRA_CONSENT = "consent_confirmed"

        /** 録音を始める。`consentConfirmed` は、参加者の同意を確認したか（false なら始まらない）。 */
        fun start(context: Context, consentConfirmed: Boolean) {
            context.startForegroundService(
                Intent(context, RecorderService::class.java).setAction(ACTION_START).putExtra(EXTRA_CONSENT, consentConfirmed),
            )
        }

        fun stop(context: Context) {
            context.startService(Intent(context, RecorderService::class.java).setAction(ACTION_STOP))
        }
    }
}
