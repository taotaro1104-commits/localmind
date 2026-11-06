package com.localmind.sample

import android.content.Context
import java.io.File
import java.io.FileOutputStream

/** 展開先の余裕として見込む空き容量（コピー中の一時ファイルと、他の保存分）。 */
private const val FREE_SPACE_MARGIN = 64L * 1024 * 1024

/**
 * assets の `assetDir` に同梱したモデル（`files`）を、filesDir の `model/<assetDir>` へ展開する
 * （長さが同じファイルは再展開しない）。内容の検証（SHA-256）は Rust 側が読み込み時に行う。長さが同じで
 * 中身だけ壊れている場合は、呼び出し側が `force = true` で作り直す。
 *
 * - 一時ファイルに書き、`fsync` してから置き換える。失敗したら一時ファイルを消す。
 * - 空き容量が足りないときは、書き始める前に分かりやすいエラーにする。
 */
fun installAssets(context: Context, assetDir: String, files: List<String>, force: Boolean = false): File {
    val names = context.assets.list(assetDir).orEmpty().toSet()
    val missing = files.filter { it !in names }
    check(missing.isEmpty()) {
        t("assets に $assetDir/${missing.joinToString()} がありません（README の手順でモデルを置いてビルドし直してください）", "$assetDir/${missing.joinToString()} is missing from assets (put the models in place as the README says, then build again)")
    }
    val dir = File(context.filesDir, "model/$assetDir")
    if (force) dir.deleteRecursively()
    dir.mkdirs()
    val sizes = files.associateWith { name ->
        context.assets.openFd("$assetDir/$name").use { it.length }
    }
    val needed = files.sumOf { name ->
        val have = File(dir, name).takeIf { it.isFile }?.length()
        if (have == sizes.getValue(name)) 0L else sizes.getValue(name)
    }
    check(needed == 0L || dir.usableSpace >= needed + FREE_SPACE_MARGIN) {
        t("保存領域が足りません（約 ${(needed + FREE_SPACE_MARGIN) / (1024 * 1024)}MB 必要）", "Not enough storage (about ${(needed + FREE_SPACE_MARGIN) / (1024 * 1024)} MB needed)")
    }
    for (name in files) {
        val target = File(dir, name)
        if (target.isFile && target.length() == sizes.getValue(name)) continue
        val tmp = File(dir, "$name.tmp")
        try {
            context.assets.open("$assetDir/$name").use { input ->
                FileOutputStream(tmp).use { out ->
                    input.copyTo(out, bufferSize = 1 shl 20)
                    out.fd.sync()
                }
            }
            check(tmp.renameTo(target)) { t("$name を置けません", "Could not place $name") }
        } finally {
            tmp.delete() // 失敗したときの断片を残さない（成功時は rename 済みで何もしない）
        }
    }
    return dir
}
