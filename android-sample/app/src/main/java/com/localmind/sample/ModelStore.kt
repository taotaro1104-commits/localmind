package com.localmind.sample

import android.content.Context
import java.io.File

/** 候補から選べるモデル。`sha256` と `bytes` は、取得元（Hugging Face）が公開している値で、固定する。 */
data class ModelSpec(val label: String, val url: String, val bytes: Long, val sha256: String) {
    val fileName: String get() = url.substringAfterLast('/')
}

/** コンボボックスに出す候補。いずれも ChatML 形式の Qwen2.5 Instruct（Apache-2.0）。 */
internal val MODEL_CATALOG = listOf(
    ModelSpec(
        "Qwen2.5 1.5B（標準・約 1.1GB）",
        "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        1_117_320_736L,
        "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
    ),
    ModelSpec(
        "Qwen2.5 0.5B（軽い・約 0.5GB・精度は低め）",
        "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        491_400_032L,
        "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
    ),
    ModelSpec(
        "Qwen2.5 3B（高精度・約 2.1GB・メモリ 3GB 以上）",
        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
        2_104_932_768L,
        "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d",
    ),
)

/** 使うと決めたモデル。 */
data class ActiveModel(val file: File, val sha256: String)

/**
 * 取得したモデルの置き場所（`files/llm/`）と、今どれを使うかの記録（`active.txt` に `ファイル名|SHA-256`）。
 * 記録の中身は信用せず、読むたびに名前と形を確かめる。
 */
object ModelStore {
    private const val DIR = "llm"
    private const val ACTIVE = "active.txt"
    private val NAME = Regex("^[A-Za-z0-9._-]{1,100}\\.gguf$")
    private val HEX64 = Regex("^[0-9a-f]{64}$")

    fun dir(context: Context): File = File(context.filesDir, DIR)

    internal fun isValidName(name: String) = NAME.matches(name) && !name.contains("..")
    internal fun isValidSha(sha: String) = HEX64.matches(sha)

    fun active(context: Context): ActiveModel? = active(dir(context))

    /** 使うモデル。無ければ null。記録が無くても、以前の標準のファイルがあればそれを使う。 */
    internal fun active(d: File): ActiveModel? {
        val line = File(d, ACTIVE).takeIf { it.isFile && it.length() < 512 }?.readText()?.trim()
        if (line != null) {
            val (name, sha) = line.split('|', limit = 2).let { it.getOrNull(0).orEmpty() to it.getOrNull(1).orEmpty() }
            val f = File(d, name)
            if (isValidName(name) && isValidSha(sha) && f.isFile) return ActiveModel(f, sha)
        }
        val legacy = File(d, MODEL_CATALOG[0].fileName)
        return if (legacy.isFile) ActiveModel(legacy, MODEL_CATALOG[0].sha256) else null
    }

    /** `file` を使うモデルにする。ほかのモデルは、新しいモデルで議事録を作れたあとに [prune] で消す。 */
    fun activate(context: Context, file: File, sha256: String) = activate(dir(context), file, sha256)

    internal fun activate(d: File, file: File, sha256: String) {
        require(isValidName(file.name) && isValidSha(sha256)) { "モデルの記録が不正です" }
        val tmp = File(d, "$ACTIVE.tmp")
        try {
            tmp.writeText("${file.name}|$sha256")
            try {
                // 既にある記録を置き換える（rename は、OS によっては既存のファイルがあると失敗する）
                java.nio.file.Files.move(
                    tmp.toPath(), File(d, ACTIVE).toPath(),
                    java.nio.file.StandardCopyOption.REPLACE_EXISTING, java.nio.file.StandardCopyOption.ATOMIC_MOVE,
                )
            } catch (_: java.io.IOException) {
                throw IllegalStateException("モデルの記録を保存できません")
            }
        } finally {
            tmp.delete() // 成功したあとは、もう無い。失敗したときの取り残しを消す
        }
    }

    /** 記録を外す（読み込めないモデルを使い続けない）。ファイルは消さない。 */
    fun forget(context: Context) {
        File(dir(context), ACTIVE).delete()
    }

    /** 使っているモデル以外の `.gguf` と、取りかけを消す（容量のため）。 */
    fun prune(context: Context) = prune(dir(context), alsoParts = !ModelDownloader.isRunning)

    /** `alsoParts` が false のときは、取りかけ（実行中のダウンロード）は消さない。 */
    internal fun prune(d: File, alsoParts: Boolean) {
        val keep = active(d)?.file?.name ?: return
        d.listFiles()
            ?.filter { it.isFile && it.name != keep && (it.name.endsWith(".gguf") || (alsoParts && it.name.endsWith(".part"))) }
            ?.forEach { it.delete() }
    }
}
