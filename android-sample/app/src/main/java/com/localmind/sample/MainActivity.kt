package com.localmind.sample

import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.WindowInsetsController
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
        actionBar?.hide()
        val dashboard = Dashboard(this)
        setContentView(dashboard.view)
        // 色付きの帯の上でも読めるよう、ステータスバーの文字は明るくする（API 30 以上）。
        if (Build.VERSION.SDK_INT >= 30) {
            window.insetsController?.setSystemBarsAppearance(0, WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS)
        }
        if (!started.compareAndSet(false, true)) {
            dashboard.notice("自己テストは実行済みです。アプリを終了してから、もう一度起動してください。")
            return
        }

        val ortError = setupOrtEnv(this)
        val debuggable = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        val appContext = applicationContext

        thread(name = "lm-selftest") {
            fun stage(title: String, run: () -> List<Check>) {
                runOnUiThread { dashboard.begin(title) }
                val results = try {
                    run()
                } catch (e: Throwable) {
                    // ライブラリのロード失敗（.so の ABI 違いなど）もここに来る。
                    listOf(Check("実行", false, "${e::class.simpleName}: ${e.message}"))
                }
                if (debuggable) {
                    results.forEach { Log.i(TAG, "${if (it.ok) "PASS" else "FAIL"} ${it.name} ${it.detail}") }
                    val failed = results.count { !it.ok }
                    Log.i(TAG, if (failed == 0) "$title: ALL PASS (${results.size})" else "$title: FAILED $failed/${results.size}")
                }
                runOnUiThread { dashboard.finish(title, results) }
            }
            stage("基本") { runSelfTest(appContext.filesDir) }
            stage("実モデル(e5)") {
                if (ortError != null) listOf(Check("ORT_DYLIB_PATH の設定", false, ortError)) else runE5Test(appContext)
            }
            runOnUiThread {
                dashboard.done(
                    onOpenSearch = { startActivity(Intent(this, SearchActivity::class.java)) },
                    onOpenRecord = { startActivity(Intent(this, RecordActivity::class.java)) },
                )
            }
        }
    }
}
