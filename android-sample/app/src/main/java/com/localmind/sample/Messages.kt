package com.localmind.sample

import android.util.Log
import uniffi.lm_ffi.FfiException

/**
 * 画面に出す失敗の文言。例外のメッセージをそのまま出さない（OS や Rust の例外には、ファイルの場所などの
 * 内部情報が入りうる）。このアプリが書いた入力の不備の文言（`IllegalArgumentException` など）と、
 * Rust 側のワークスペースのエラー（パスを含まない文言にしてある）だけを、そのまま出す。
 * 詳細はデバッグビルドのときだけ logcat に出す。
 */
fun userMessage(e: Throwable, debuggable: Boolean, fallback: String = t("うまくいきませんでした", "Something went wrong")): String {
    if (debuggable) Log.w("LMSample", "failure: $e", e)
    return when (e) {
        is IllegalArgumentException, is IllegalStateException -> e.message ?: fallback
        is FfiException.Workspace -> ErrorText.localize(e.msg) ?: fallback
        is FfiException.Ingest -> ErrorText.localize(e.msg) ?: fallback
        is FfiException.Integrity -> t("モデルのファイルが壊れています。アプリを入れ直してください", "The model file is damaged. Please reinstall the app")
        is SecurityException -> t("このファイルを読む許可がありません", "No permission to read this file")
        is java.io.IOException -> t("ファイルを読み込めませんでした", "Could not read the file")
        else -> fallback
    }
}
