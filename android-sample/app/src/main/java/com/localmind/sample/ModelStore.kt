package com.localmind.sample

import android.content.Context
import java.io.File

/** 候補から選べるモデル。`sha256` と `bytes` は、取得元（Hugging Face）が公開している値で、固定する。 */
data class ModelSpec(val labelJa: String, val labelEn: String, val url: String, val bytes: Long, val sha256: String) {
    /** 画面に出す名前（いまの言語）。 */
    val label: String get() = t(labelJa, labelEn)
    val fileName: String get() = url.substringAfterLast('/')
}

/** コンボボックスに出す候補。いずれも ChatML 形式の Qwen2.5 Instruct（Apache-2.0）。 */
internal val MODEL_CATALOG = listOf(
    ModelSpec(
        "Qwen2.5 1.5B（標準・約 1.1GB）",
        "Qwen2.5 1.5B (standard, about 1.1 GB)",
        "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        1_117_320_736L,
        "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
    ),
    ModelSpec(
        "Qwen2.5 0.5B（軽い・約 0.5GB・精度は低め）",
        "Qwen2.5 0.5B (light, about 0.5 GB, lower accuracy)",
        "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        491_400_032L,
        "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
    ),
    ModelSpec(
        "Qwen2.5 3B（高精度・約 2.1GB・メモリ 3GB 以上）",
        "Qwen2.5 3B (more accurate, about 2.1 GB, needs 3 GB or more of memory)",
        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
        2_104_932_768L,
        "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d",
    ),
)

/** 使うと決めたモデル。 */
data class ActiveModel(val file: File, val sha256: String)

/** 取得済みのモデル（切り替えの候補）。 */
data class InstalledModel(val file: File, val sha256: String) {
    /** 画面に出す名前。候補のモデルは候補の名前、取り込んだモデルはファイル名。 */
    val label: String get() = MODEL_CATALOG.firstOrNull { it.sha256 == sha256 }?.label ?: t("取り込んだモデル（${file.name}）", "Imported model (${file.name})")
    val megabytes: Long get() = file.length() / (1024 * 1024)
}

/**
 * 取得したモデルの置き場所（`files/llm/`）と、今どれを使うかの記録（`active.txt` に `ファイル名|SHA-256`）。
 * 取得したモデルの一覧（`installed.txt` に、1 行ずつ `ファイル名|SHA-256`）も持ち、切り替えと削除に使う。
 * 記録の中身は信用せず、読むたびに名前と形を確かめる。
 */
object ModelStore {
    private const val DIR = "llm"
    private const val ACTIVE = "active.txt"
    private const val INSTALLED = "installed.txt"
    private const val MAX_RECORD_BYTES = 16 * 1024
    private val NAME = Regex("^[A-Za-z0-9._-]{1,100}\\.gguf$")
    private val HEX64 = Regex("^[0-9a-f]{64}$")

    fun dir(context: Context): File = File(context.filesDir, DIR)

    internal fun isValidName(name: String) = NAME.matches(name) && !name.contains("..")
    internal fun isValidSha(sha: String) = HEX64.matches(sha)

    fun active(context: Context): ActiveModel? = active(dir(context))

    /** 使っているモデルの名前（画面に出す）。無ければ null。 */
    fun activeLabel(context: Context): String? = active(context)?.let { InstalledModel(it.file, it.sha256).label }

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

    /** `file` を使うモデルにして、取得済みの一覧にも入れる。ほかのモデルは消さない（切り替えで選び直せる）。 */
    fun activate(context: Context, file: File, sha256: String) = activate(dir(context), file, sha256)

    @Synchronized
    internal fun activate(d: File, file: File, sha256: String) {
        require(isValidName(file.name) && isValidSha(sha256)) { t("モデルの記録が不正です", "The model record is invalid") }
        // 一覧を先に更新する（途中で失敗しても、使っているモデルは一覧にある状態を保つ）
        // 登録簿に無かった使用中のモデル（以前の標準のファイルなど）も、一覧から落とさずに引き継ぐ
        val others = installed(d).map { it.file.name to it.sha256 }.filter { it.first != file.name }
        writeAtomic(d, INSTALLED, (others + (file.name to sha256)).joinToString("\n") { "${it.first}|${it.second}" })
        writeAtomic(d, ACTIVE, "${file.name}|$sha256")
    }

    /** 記録を、一時ファイル経由で置き換える（rename は、OS によっては既存のファイルがあると失敗する）。 */
    private fun writeAtomic(d: File, name: String, content: String) {
        val tmp = File(d, "$name.tmp")
        try {
            tmp.writeText(content)
            try {
                java.nio.file.Files.move(
                    tmp.toPath(), File(d, name).toPath(),
                    java.nio.file.StandardCopyOption.REPLACE_EXISTING, java.nio.file.StandardCopyOption.ATOMIC_MOVE,
                )
            } catch (_: java.io.IOException) {
                throw IllegalStateException(t("モデルの記録を保存できません", "Cannot save the model record"))
            }
        } finally {
            tmp.delete() // 成功したあとは、もう無い。失敗したときの取り残しを消す
        }
    }

    /** 一覧の記録（形の正しい行だけ）。 */
    private fun readInstalled(d: File): List<Pair<String, String>> =
        File(d, INSTALLED).takeIf { it.isFile && it.length() < MAX_RECORD_BYTES }?.readLines().orEmpty().mapNotNull { line ->
            val (name, sha) = line.trim().split('|', limit = 2).let { it.getOrNull(0).orEmpty() to it.getOrNull(1).orEmpty() }
            (name to sha).takeIf { isValidName(name) && isValidSha(sha) }
        }

    fun installed(context: Context): List<InstalledModel> = installed(dir(context))

    /**
     * 取得済みのモデル（ファイルがあるものだけ。名前順）。一覧の記録のほか、使っているモデルと、以前の標準のファイルも含める
     * （一覧ができる前に取得したものを、切り替えの候補から落とさない）。
     */
    internal fun installed(d: File): List<InstalledModel> {
        val known = linkedMapOf<String, String>()
        readInstalled(d).forEach { (name, sha) -> known[name] = sha }
        active(d)?.let { known[it.file.name] = it.sha256 }
        return known.entries
            .map { InstalledModel(File(d, it.key), it.value) }
            .filter { it.file.isFile }
            .sortedBy { it.file.name }
    }

    /** 取得済みのモデルの 1 つを、使うモデルにする。無ければ例外。 */
    fun switchTo(context: Context, name: String) = switchTo(dir(context), name)

    @Synchronized
    internal fun switchTo(d: File, name: String) {
        check(!ModelGate.isBusy() && !ModelDownloader.isRunning) { t("モデルを使う処理の最中は、切り替えられません", "You cannot switch while a task that uses the model is running") }
        val m = installed(d).firstOrNull { it.file.name == name } ?: throw IllegalStateException(t("そのモデルは見つかりません", "That model was not found"))
        activate(d, m.file, m.sha256)
    }

    /** 取得済みのモデルを消す。使っているモデルは消せない（使うモデルを先に切り替える）。 */
    fun remove(context: Context, name: String) = remove(dir(context), name)

    @Synchronized
    internal fun remove(d: File, name: String) {
        check(!ModelGate.isBusy() && !ModelDownloader.isRunning) { t("モデルを使う処理の最中は、削除できません", "You cannot delete while a task that uses the model is running") }
        val m = installed(d).firstOrNull { it.file.name == name } ?: throw IllegalStateException(t("そのモデルは見つかりません", "That model was not found"))
        check(active(d)?.file?.name != name) { t("使っているモデルは削除できません。先に別のモデルへ切り替えてください", "The model in use cannot be deleted. Switch to another model first") }
        check(m.file.delete() || !m.file.exists()) { t("モデルを削除できませんでした", "Could not delete the model") }
        writeAtomic(d, INSTALLED, readInstalled(d).filter { it.first != name }.joinToString("\n") { "${it.first}|${it.second}" })
    }

    /** 記録を外す（読み込めないモデルを使い続けない）。ファイルは消さない。 */
    fun forget(context: Context) {
        File(dir(context), ACTIVE).delete()
    }

    /**
     * 取りかけ（`.part`）だけを消す。`.gguf` は、記録が壊れていても・取得の途中でも、決して消さない
     * （モデルを消すのは、利用者が [remove] で選んだときだけ）。
     */
    fun prune(context: Context) = prune(dir(context), alsoParts = !ModelDownloader.isRunning)

    /** `alsoParts` が false のときは、取りかけ（実行中のダウンロード）は消さない。 */
    internal fun prune(d: File, alsoParts: Boolean) {
        if (!alsoParts) return
        d.listFiles()?.filter { it.isFile && it.name.endsWith(".part") }?.forEach { it.delete() }
    }
}
