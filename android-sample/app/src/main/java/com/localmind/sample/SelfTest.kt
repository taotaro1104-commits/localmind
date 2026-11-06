package com.localmind.sample

import java.io.File
import uniffi.lm_ffi.DeviceProfile
import uniffi.lm_ffi.FfiException
import uniffi.lm_ffi.ForeignEmbedder
import uniffi.lm_ffi.Route
import uniffi.lm_ffi.SearchConfig
import uniffi.lm_ffi.SearchEngine
import uniffi.lm_ffi.Task
import uniffi.lm_ffi.decideRoute
import uniffi.lm_ffi.e5SearchConfig

/** 1 項目の結果。 */
data class Check(val name: String, val ok: Boolean, val detail: String = "")

/** Kotlin 側の埋め込み（実モデルの代わり）。文字のハッシュから 8 次元のベクトルを作る。 */
private class ToyEmbedder(private val failDim: Boolean = false) : ForeignEmbedder {
    override fun embedDocument(text: String): List<Float> = vec(text)
    override fun embedQuery(text: String): List<Float> = vec(text)
    override fun dim(): UInt = if (failDim) throw IllegalStateException("dim failed") else 8u
    override fun id(): String = "toy-embedder-v1"

    private fun vec(text: String): List<Float> {
        val v = FloatArray(8)
        text.forEach { v[it.code % 8] += 1f }
        return v.toList()
    }
}

private inline fun check(name: String, block: () -> String?): Check =
    try {
        val failure = block()
        Check(name, failure == null, failure ?: "")
    } catch (e: Throwable) {
        Check(name, false, "${e::class.simpleName}: ${e.message}")
    }

/** Rust コア（liblm_ffi.so）を端から端まで呼ぶ自己テスト。`dir` はアプリ専用領域。 */
fun runSelfTest(dir: File): List<Check> {
    val results = mutableListOf<Check>()
    fun profile(nano: Boolean, gguf: Boolean) =
        DeviceProfile(ramMb = 8192u, nanoAvailable = nano, pcReachable = false, ggufInstalled = gguf)

    results += check("経路: Nano があれば短文は Nano") {
        decideRoute(Task.SHORT_TEXT, profile(true, true)).takeIf { it != Route.NANO }?.let { "got $it" }
    }
    results += check("経路: Nano が無ければ GGUF へ縮退") {
        decideRoute(Task.SHORT_TEXT, profile(false, true)).takeIf { it != Route.GGUF }?.let { "got $it" }
    }
    results += check("経路: GGUF が無ければ Kotlin 例外（FfiException.Route）") {
        try {
            decideRoute(Task.DOCUMENT_RAG, profile(false, false))
            "例外が出なかった"
        } catch (e: FfiException.Route) {
            null
        }
    }

    val lexical = File(dir, "lexical.db").also { it.delete() }
    results += check("検索: 登録して引ける・無関係な質問は見つかりません") {
        SearchEngine.openLexical(lexical.absolutePath).use { e ->
            e.upsert("a", "契約書の解除は第12条に定める")
            e.upsert("b", "議事録の保存期間は30日")
            val hit = e.search("第12条", 3u)
            val none = e.search("宇宙旅行の費用", 3u)
            when {
                e.len() != 2uL -> "len=${e.len()}"
                !hit.found || hit.hits.firstOrNull()?.id != "a" -> "hit=$hit"
                none.found -> "none=$none"
                else -> null
            }
        }
    }
    results += check("検索: 再オープンしても索引が残る（永続化）") {
        SearchEngine.openLexical(lexical.absolutePath).use { e ->
            if (e.len() != 2uL) "len=${e.len()}" else null
        }
    }

    results += check("埋め込み注入: Kotlin 実装でも登録・検索できる") {
        val db = File(dir, "toy.db").also { it.delete() }
        SearchEngine.open(db.absolutePath, ToyEmbedder(), e5SearchConfig()).use { e ->
            e.upsert("a", "予算案を承認した")
            e.search("予算案", 3u)
            if (e.isEmpty()) "登録されていない" else null
        }
    }
    results += check("埋め込み注入: dim() の例外は FfiException になる（クラッシュしない）") {
        val db = File(dir, "toy2.db").also { it.delete() }
        try {
            SearchEngine.open(db.absolutePath, ToyEmbedder(failDim = true), e5SearchConfig()).close()
            "例外が出なかった"
        } catch (e: FfiException) {
            null
        }
    }
    results += check("入力検証: 相対パスは拒否") {
        try {
            SearchEngine.openLexical("relative.db").close()
            "拒否されなかった"
        } catch (e: FfiException.Search) {
            null
        }
    }
    results += check("入力検証: 不正な設定（NaN）は open で拒否") {
        val db = File(dir, "cfg.db").also { it.delete() }
        try {
            SearchEngine.open(db.absolutePath, ToyEmbedder(), SearchConfig(Float.NaN, 0f)).close()
            "拒否されなかった"
        } catch (e: FfiException.Search) {
            null
        }
    }
    results += check("モデル取得: 接続先は https の許可したホストだけ") {
        val ok = listOf("https://huggingface.co/a", "https://us.aws.cdn.hf.co/x?y=1")
        val bad = listOf(
            "http://hf.co/x", "https://hf.co/x", "https://evilhf.co/x", "https://hf.co.evil.com/x",
            "https://user:pw@hf.co/x", "https://hf.co@evil.com/x", "https://evil.com/hf.co",
        )
        val wrong = ok.filterNot { ModelDownloader.isSafeRedirect(java.net.URL(it)) } +
            bad.filter { ModelDownloader.isSafeRedirect(java.net.URL(it)) }
        if (wrong.isEmpty()) null else "判定が違う: ${wrong.size} 件"
    }
    results += check("モデル取得: 直接入れた URL は https のみ・認証情報なし・SHA-256 は書式を確かめる") {
        fun accepted(url: String, sha: String? = null) = try { ModelSource.custom(url, sha); true } catch (_: ModelDownloadException) { false }
        val wrong = listOf(
            accepted("https://example.com/m.gguf"),
            !accepted("http://example.com/m.gguf"),
            !accepted("https://user:pw@example.com/m.gguf"),
            !accepted("ftp://example.com/m.gguf"),
            !accepted("file:///data/m.gguf"),
            !accepted("not a url"),
            !accepted("https://example.com/m.gguf", "xyz"),
            accepted("https://example.com/m.gguf", "a".repeat(64)),
        ).count { !it }
        if (wrong == 0) null else "判定が違う: $wrong 件"
    }
    results += check("モデル取得: 保存するファイル名はパスを含まない .gguf だけ") {
        val ok = listOf("qwen2.5-1.5b-instruct-q4_k_m.gguf", "custom-0123456789ab.gguf")
        val bad = listOf("../x.gguf", "a/b.gguf", "..gguf", "x.bin", "", "a\b.gguf", ".." + ".gguf")
        val wrong = ok.filterNot { ModelStore.isValidName(it) } + bad.filter { ModelStore.isValidName(it) }
        if (wrong.isEmpty()) null else "判定が違う: ${wrong.size} 件"
    }
    results += check("モデル取得: LAN・自端末のアドレスは、リダイレクト先として断る") {
        val local = listOf("127.0.0.1", "10.0.0.5", "192.168.1.1", "172.16.0.9", "169.254.1.1", "[::1]", "[fd00::1]", "0.0.0.0")
        val wrong = local.filterNot { ModelDownloader.isLocalAddress(it) } + listOf("8.8.8.8").filter { ModelDownloader.isLocalAddress(it) }
        if (wrong.isEmpty()) null else "判定が違う: $wrong"
    }
    results += check("モデルの記録: 壊れた記録は使わず、切り替えと掃除が働く") {
        val d = File(dir, "models").also { it.deleteRecursively(); it.mkdirs() }
        val shaA = "a".repeat(64)
        val shaB = "b".repeat(64)
        val a = File(d, "a.gguf").also { it.writeText("x") }
        val b = File(d, "b.gguf").also { it.writeText("y") }
        val part = File(d, "dl-0123456789ab.part").also { it.writeText("z") }
        val errors = mutableListOf<String>()
        if (ModelStore.active(d) != null) errors += "空なのに有効"
        File(d, "active.txt").writeText("../a.gguf|$shaA")
        if (ModelStore.active(d) != null) errors += "パスを含む記録を受け入れた"
        File(d, "active.txt").writeText("a.gguf|xyz")
        if (ModelStore.active(d) != null) errors += "不正な SHA の記録を受け入れた"
        ModelStore.activate(d, a, shaA)
        if (ModelStore.active(d)?.sha256 != shaA) errors += "記録を読み戻せない"
        ModelStore.activate(d, b, shaB)
        if (ModelStore.active(d)?.file?.name != "b.gguf" || !a.exists()) errors += "切り替えで、前のモデルが先に消えた"
        ModelStore.prune(d, alsoParts = false)
        if (a.exists() || !part.exists()) errors += "掃除（取りかけを残す）が違う"
        ModelStore.prune(d, alsoParts = true)
        if (part.exists() || !b.exists()) errors += "掃除（取りかけも消す）が違う"
        d.deleteRecursively()
        if (errors.isEmpty()) null else errors.joinToString("、")
    }
    return results
}
