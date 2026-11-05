package com.localmind.sample

import android.util.Log
import uniffi.lm_ffi.FfiException

/**
 * 画面に出す失敗の文言。例外のメッセージをそのまま出さない（OS や Rust の例外には、ファイルの場所などの
 * 内部情報が入りうる）。このアプリが書いた入力の不備の文言（`IllegalArgumentException` など）と、
 * Rust 側のワークスペースのエラー（パスを含まない文言にしてある）だけを、そのまま出す。
 * 詳細はデバッグビルドのときだけ logcat に出す。
 */
fun userMessage(e: Throwable, debuggable: Boolean, fallback: String = "うまくいきませんでした"): String {
    if (debuggable) Log.w("LMSample", "failure: $e", e)
    return when (e) {
        is IllegalArgumentException, is IllegalStateException -> e.message ?: fallback
        is FfiException.Workspace -> e.message ?: fallback
        is FfiException.Integrity -> "モデルのファイルが壊れています。アプリを入れ直してください"
        is SecurityException -> "このファイルを読む許可がありません"
        is java.io.IOException -> "ファイルを読み込めませんでした"
        else -> fallback
    }
}
