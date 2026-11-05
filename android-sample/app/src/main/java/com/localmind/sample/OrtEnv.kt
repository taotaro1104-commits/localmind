package com.localmind.sample

import android.content.Context
import android.system.Os
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

private val done = AtomicBoolean(false)

/**
 * Rust 側（ort）が実行時に読む ONNX Runtime の場所を、環境変数で教える。プロセスで 1 度だけ、
 * 他のスレッドを作る前に呼ぶこと（setenv はスレッドセーフではない）。失敗したときは理由を返す。
 */
fun setupOrtEnv(context: Context): String? {
    if (!done.compareAndSet(false, true)) return null
    return try {
        Os.setenv("ORT_DYLIB_PATH", File(context.applicationInfo.nativeLibraryDir, "libonnxruntime.so").path, true)
        null
    } catch (e: Exception) {
        done.set(false)
        "${e::class.simpleName}: ${e.message}"
    }
}
