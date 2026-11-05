package com.localmind.sample

import android.app.Activity
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.system.Os
import android.util.Log
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

private const val TAG = "LMSample"

/** プロセス内で自己テストを 1 度だけ走らせる（画面の作り直しで 2 本目が走らないように）。 */
private val started = AtomicBoolean(false)

/**
 * 起動すると自己テスト（軽い順に 2 段階）を実行し、結果を画面と logcat（タグ LMSample）に出す。
 * logcat への出力はデバッグビルドのときだけ（エラー文にはアプリ内のパスが入るため）。
 */
class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val text = TextView(this).apply {
            setPadding(32, 48, 32, 48)
            textSize = 14f
            setTextIsSelectable(true)
            this.text = "実行中…（実モデルの展開と読み込みに時間がかかります）"
        }
        setContentView(ScrollView(this).apply { addView(text) })
        if (!started.compareAndSet(false, true)) {
            text.text = "自己テストは実行済みです（アプリを終了してからもう一度起動してください）。"
            return
        }

        // Rust 側（ort）が実行時に読む ONNX Runtime。他のスレッドが動く前に、ここで 1 度だけ設定する
        // （setenv はスレッドセーフではない）。AAR が nativeLibraryDir に置く。
        val ortError = try {
            Os.setenv("ORT_DYLIB_PATH", File(applicationInfo.nativeLibraryDir, "libonnxruntime.so").path, true)
            null
        } catch (e: Exception) {
            "${e::class.simpleName}: ${e.message}"
        }
        val debuggable = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        val appContext = applicationContext

        thread(name = "lm-selftest") {
            val lines = mutableListOf<String>()
            fun stage(title: String, run: () -> List<Check>) {
                val results = try {
                    run()
                } catch (e: Throwable) {
                    // ライブラリのロード失敗（.so の ABI 違いなど）もここに来る。
                    listOf(Check("実行", false, "${e::class.simpleName}: ${e.message}"))
                }
                results.forEach {
                    if (debuggable) Log.i(TAG, "${if (it.ok) "PASS" else "FAIL"} ${it.name} ${it.detail}")
                    lines += "${if (it.ok) "✔" else "✘"} ${it.name}" +
                        if (it.detail.isNotEmpty()) "\n    ${it.detail}" else ""
                }
                val failed = results.count { !it.ok }
                val summary = if (failed == 0) "$title: ALL PASS (${results.size})" else "$title: FAILED $failed/${results.size}"
                if (debuggable) Log.i(TAG, summary)
                lines += "\n$summary\n"
                runOnUiThread { text.text = lines.joinToString("\n") }
            }
            stage("基本") { runSelfTest(appContext.filesDir) }
            stage("実モデル(e5)") {
                if (ortError != null) listOf(Check("ORT_DYLIB_PATH の設定", false, ortError)) else runE5Test(appContext)
            }
        }
    }
}
