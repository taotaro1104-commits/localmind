package com.localmind.sample

import android.content.Context
import java.io.File
import java.io.FileOutputStream
import uniffi.lm_ffi.FfiException
import uniffi.lm_ffi.SearchEngine
import uniffi.lm_ffi.e5SearchConfig

private const val ASSET_DIR = "e5-small"

/** assets に同梱されているべきモデルのファイル（Rust 側の固定リストと同じ名前）。 */
private val MODEL_FILES = listOf(
    "onnx__model.onnx",
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
)

/** 展開先の余裕として見込む空き容量（コピー中の一時ファイルと、他の保存分）。 */
private const val FREE_SPACE_MARGIN = 64L * 1024 * 1024

/**
 * assets に同梱したモデルを filesDir へ展開する（長さが同じファイルは再展開しない）。
 * 内容の検証（SHA-256）は Rust 側が読み込み時に行う。長さが同じで中身だけ壊れている場合は、
 * 呼び出し側が `force = true` で作り直す。
 *
 * - 一時ファイルに書き、`fsync` してから置き換える。失敗したら一時ファイルを消す。
 * - 空き容量が足りないときは、書き始める前に分かりやすいエラーにする。
 */
fun installModel(context: Context, force: Boolean = false): File {
    val names = context.assets.list(ASSET_DIR).orEmpty().toSet()
    val missing = MODEL_FILES.filter { it !in names }
    check(missing.isEmpty()) {
        "assets に $ASSET_DIR/${missing.joinToString()} がありません（README の手順でモデルを置いてビルドし直してください）"
    }
    val dir = File(context.filesDir, "model/$ASSET_DIR")
    if (force) dir.deleteRecursively()
    dir.mkdirs()
    val sizes = MODEL_FILES.associateWith { name ->
        context.assets.openFd("$ASSET_DIR/$name").use { it.length }
    }
    val needed = MODEL_FILES.sumOf { name ->
        val have = File(dir, name).takeIf { it.isFile }?.length()
        if (have == sizes.getValue(name)) 0L else sizes.getValue(name)
    }
    check(needed == 0L || dir.usableSpace >= needed + FREE_SPACE_MARGIN) {
        "保存領域が足りません（約 ${(needed + FREE_SPACE_MARGIN) / (1024 * 1024)}MB 必要）"
    }
    for (name in MODEL_FILES) {
        val target = File(dir, name)
        if (target.isFile && target.length() == sizes.getValue(name)) continue
        val tmp = File(dir, "$name.tmp")
        try {
            context.assets.open("$ASSET_DIR/$name").use { input ->
                FileOutputStream(tmp).use { out ->
                    input.copyTo(out, bufferSize = 1 shl 20)
                    out.fd.sync()
                }
            }
            check(tmp.renameTo(target)) { "$name を置けません" }
        } finally {
            tmp.delete() // 失敗したときの断片を残さない（成功時は rename 済みで何もしない）
        }
    }
    return dir
}

private inline fun timed(block: () -> String?): Pair<String?, Long> {
    val t = System.nanoTime()
    val r = try {
        block()
    } catch (e: Exception) { // OutOfMemoryError などの Error は握りつぶさず上位へ出す
        "${e::class.simpleName}: ${e.message}"
    }
    return r to (System.nanoTime() - t) / 1_000_000
}

/**
 * 同梱の実モデル（multilingual-e5-small）で、日本語の検索と「見つかりません」を確かめる。
 * 前提: 環境変数 `ORT_DYLIB_PATH` を設定済み（`MainActivity` がスレッドを作る前に行う）。
 */
fun runE5Test(context: Context): List<Check> {
    val results = mutableListOf<Check>()
    fun run(name: String, block: () -> String?): Boolean {
        val (failure, ms) = timed(block)
        results += Check("$name (${ms}ms)", failure == null, failure ?: "")
        return failure == null
    }

    var dir: File? = null
    if (!run("モデルを filesDir へ展開") { dir = installModel(context); null }) return results
    val modelDir = dir ?: return results

    val db = File(context.filesDir, "e5.db").also { it.delete() }
    var engine: SearchEngine? = null
    run("実モデルを読み込む（SHA-256 検証 + 初期化）") {
        engine = try {
            SearchEngine.openE5(db.absolutePath, modelDir.absolutePath, e5SearchConfig())
        } catch (e: FfiException.Embedding) {
            // 展開済みのファイルが壊れている（長さは同じで中身が違う等）なら、作り直して 1 度だけやり直す。
            SearchEngine.openE5(
                db.absolutePath,
                installModel(context, force = true).absolutePath,
                e5SearchConfig(),
            )
        }
        null
    }
    val e = engine ?: return results
    e.use {
        run("文書を登録（4 件・埋め込み）") {
            e.upsert("contract", "契約書の解除は第12条に定める。解除するには30日前までに書面で通知する。")
            e.upsert("minutes", "議事録の保存期間は30日とし、期限を過ぎた録音は自動的に削除される。")
            e.upsert("budget", "来期の予算案は営業部の増員を含めて承認された。")
            e.upsert("travel", "出張の経費精算は帰着後5営業日以内に申請する。")
            if (e.len() != 4uL) "len=${e.len()}" else null
        }
        run("意味検索: 言い換えた質問で正しい文書が 1 位") {
            val r = e.search("契約をやめたいときはどうすればいいですか", 3u)
            if (!r.found || r.hits.firstOrNull()?.id != "contract") "$r" else null
        }
        run("意味検索: 別の質問で別の文書が 1 位") {
            val r = e.search("録音データはいつ消えますか", 3u)
            if (!r.found || r.hits.firstOrNull()?.id != "minutes") "$r" else null
        }
        run("無関係な質問は「見つかりません」") {
            val r = e.search("宇宙旅行の費用はいくらですか", 3u)
            if (r.found) "$r" else null
        }
    }
    // 再オープンは、先のエンジンを閉じてから行う（モデルを同時に 2 つ読むとメモリが倍になる）。
    run("再オープンしても索引が残る（モデルの読み込みを含む）") {
        SearchEngine.openE5(db.absolutePath, modelDir.absolutePath, e5SearchConfig()).use {
            if (it.len() != 4uL) "len=${it.len()}" else null
        }
    }
    return results
}
