package com.localmind.sample

import android.content.Context
import java.io.File
import java.util.concurrent.atomic.AtomicReference
import uniffi.lm_ffi.RecordingStore

/** 録音の設定（端末内にだけ保存する）。 */
class RecordPrefs(context: Context) {
    private val sp = context.getSharedPreferences("record", Context.MODE_PRIVATE)

    /** 録音を始める前に、参加者の同意を確認する。既定は確認する。 */
    var confirmConsent: Boolean
        get() = sp.getBoolean("confirm_consent", true)
        set(v) = sp.edit().putBoolean("confirm_consent", v).apply()

    /** 録音の保存期間（日）。過ぎた録音は自動で消える。 */
    var retentionDays: Int
        get() = sp.getInt("retention_days", DEFAULT_RETENTION_DAYS).coerceIn(1, MAX_RETENTION_DAYS)
        set(v) = sp.edit().putInt("retention_days", v.coerceIn(1, MAX_RETENTION_DAYS)).apply()

    companion object {
        const val DEFAULT_RETENTION_DAYS = 30
        const val MAX_RETENTION_DAYS = 365
        val CHOICES = listOf(7, 30, 90)
    }
}

/** 録音のある時点の状態（変更できない。画面は 1 回の読み取りで、そろった値を得る）。 */
data class RecState(
    val active: Boolean = false,
    val startedAtMs: Long = 0,
    val savedChunks: Int = 0,
    /** 直近の失敗（画面に出したら [RecorderState.takeError] で消す）。 */
    val error: String? = null,
)

/** 録音の今の状態（サービスが更新し、画面が読む）。更新は不可分で、どのスレッドからでもよい。 */
object RecorderState {
    private val ref = AtomicReference(RecState())

    val snapshot: RecState get() = ref.get()
    val active: Boolean get() = ref.get().active

    fun begin(startedAtMs: Long) = ref.set(RecState(active = true, startedAtMs = startedAtMs))

    fun saved(chunks: Int) {
        ref.updateAndGet { it.copy(savedChunks = chunks) }
    }

    /** 録音を終える。`error` があれば残す（先に起きた失敗を、後の失敗で上書きしない）。 */
    fun end(error: String? = null) {
        ref.updateAndGet { it.copy(active = false, error = it.error ?: error) }
    }

    /** 録音の状態は変えずに、知らせたいことだけを残す（先にある失敗は上書きしない）。 */
    fun warn(message: String) {
        ref.updateAndGet { it.copy(error = it.error ?: message) }
    }

    /** 失敗を取り出して消す（取り出しと消去は不可分。読み損ねも、二重に出すこともない）。 */
    fun takeError(): String? = ref.getAndUpdate { it.copy(error = null) }.error
}

/** 録音の保存先（アプリ専用ディレクトリの中）。 */
fun openRecordingStore(context: Context): RecordingStore =
    RecordingStore.open(File(context.filesDir, "recordings").absolutePath)

/** 保持期間を過ぎた録音と、古い一時ファイルを消す。失敗したら理由を返す。 */
object Purger {
    fun run(context: Context): String? =
        runCatching {
            val store = openRecordingStore(context)
            store.purgeExpired(System.currentTimeMillis().toULong(), RecordPrefs(context).retentionDays.toUInt())
            store.cleanStaleTmp()
        }.exceptionOrNull()?.let { userMessage(it, debuggable = false, fallback = t("古い録音を消せませんでした", "Could not delete the old recordings")) }
}
