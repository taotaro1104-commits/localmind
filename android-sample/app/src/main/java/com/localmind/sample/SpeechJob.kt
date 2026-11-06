package com.localmind.sample

import android.content.Context
import java.util.concurrent.atomic.AtomicBoolean
import uniffi.lm_ffi.FfiException
import uniffi.lm_ffi.RecordingStore
import uniffi.lm_ffi.SpeechEngine
import uniffi.lm_ffi.SpeechProgress
import uniffi.lm_ffi.SpeechQuality
import uniffi.lm_ffi.SpeechResult
import uniffi.lm_ffi.SpokenSegment

/** 同梱の whisper モデル（ggml-base）。ファイルの SHA-256 は、取得元（Hugging Face）が公開している値。 */
private const val WHISPER_ASSET_DIR = "whisper-base"
private const val WHISPER_FILE = "ggml-base.bin"
private const val WHISPER_SHA256 = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"

/**
 * 保存済みの録音を文字起こしする。重いので、別スレッドから呼ぶこと。
 * 実行中かどうかと中止の指示はプロセス全体で持つ（画面を作り直しても、二重に走らず、止められる）。
 */
object SpeechJob {
    private val running = AtomicBoolean(false)
    @Volatile private var engine: SpeechEngine? = null
    @Volatile private var cancelRequested = false

    val isRunning: Boolean get() = running.get()

    /** 実行中の文字起こしを止める（推論の途中でも止まる）。何も走っていなければ何もしない。 */
    fun cancel() {
        cancelRequested = true
        engine?.cancel()
    }

    /**
     * 録音 `id` を文字起こしする。すでに実行中なら null（二重に走らせない）。進み具合は
     * `onProgress(終わったチャンク数, 全部の数)`。モデルのファイルが壊れていたら、同梱のものを入れ直して 1 度だけやり直す。
     */
    fun run(
        context: Context,
        store: RecordingStore,
        id: String,
        onProgress: (Int, Int) -> Unit,
    ): SpeechResult? {
        if (!running.compareAndSet(false, true)) return null
        cancelRequested = false
        try {
            val opened = open(context)
            engine = opened
            try {
                if (cancelRequested) opened.cancel()
                val listener = object : SpeechProgress {
                    override fun onProgress(done: UInt, total: UInt): Boolean {
                        onProgress(done.toInt(), total.toInt())
                        return !cancelRequested
                    }
                }
                return opened.transcribeRecording(store, id, listener)
            } finally {
                engine = null
                opened.close()
            }
        } finally {
            running.set(false)
        }
    }

    /** モデルを開く。内容が合わないときは、入れ直して 1 度だけやり直す。 */
    private fun open(context: Context): SpeechEngine {
        fun openOnce(force: Boolean): SpeechEngine {
            val dir = installAssets(context, WHISPER_ASSET_DIR, listOf(WHISPER_FILE), force)
            return SpeechEngine.open(
                java.io.File(dir, WHISPER_FILE).absolutePath, WHISPER_SHA256, "ja", SpeechQuality.FAST,
                Runtime.getRuntime().availableProcessors().coerceIn(1, 4).toUInt(),
            )
        }
        return try {
            openOnce(force = false)
        } catch (e: FfiException.Integrity) {
            openOnce(force = true)
        }
    }

    /** 区間を `mm:ss  本文` の行にする。 */
    fun format(segments: List<SpokenSegment>): String =
        segments.joinToString("\n") { s ->
            val sec = s.startMs.toLong() / 1000
            String.format(java.util.Locale.US, "%02d:%02d  %s", sec / 60, sec % 60, s.text)
        }
}
