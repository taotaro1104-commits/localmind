package com.localmind.sample

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.view.WindowInsetsController
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import uniffi.lm_ffi.DocFormat
import uniffi.lm_ffi.FfiException
import uniffi.lm_ffi.WorkspaceEngine
import uniffi.lm_ffi.e5SearchConfig

private const val REQUEST_PICK = 1001

/** 準備ができる前に受け付ける共有の件数の上限。 */
private const val MAX_QUEUED = 5

/**
 * 資料を日本語で検索する画面（LLM なし）。質問を入れると、根拠になる資料を近い順に出す。
 * 資料に答えが無いときは「見つかりません」と出す。資料は、入力・ファイル選択・共有シートから足せる。
 */
class SearchActivity : Activity() {
    @Volatile private var engine: WorkspaceEngine? = null
    private lateinit var screen: SearchScreen
    private val debuggable by lazy { applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0 }

    /** 準備ができる前に届いた資料（準備ができたら順に取り込む）。UI スレッドだけが触る。 */
    private val pending = ArrayDeque<Imported>()
    private val sequence = AtomicInteger()

    /** ファイル・共有の読み込みは 1 つずつ行う（外から来る入力が、スレッドを際限なく増やさないように）。 */
    private val importer = Executors.newSingleThreadExecutor { r -> Thread(r, "lm-import") }
    private var importing = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.hide()
        screen = SearchScreen(this, ::runSearch, ::addTyped, ::confirmRemove, ::pickFile)
        setContentView(screen.view)
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            window.insetsController?.setSystemBarsAppearance(0, WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS)
        }
        val ortError = setupOrtEnv(this)
        val context = applicationContext
        // 作り直し（プロセスが終了したあとの復元など）では、同じ共有をもう一度取り込まない。
        if (savedInstanceState == null) handleShare(intent)
        thread(name = "lm-search-open") {
            val result = runCatching {
                check(ortError == null) { "ONNX Runtime の準備に失敗しました" }
                val modelDir = installModel(context)
                val db = File(context.filesDir, "workspace.db").absolutePath
                val docs = File(context.filesDir, "workspace-docs").absolutePath
                val e = try {
                    WorkspaceEngine.openE5(db, docs, modelDir.absolutePath, e5SearchConfig())
                } catch (ex: FfiException.Integrity) {
                    // 展開済みのモデルが壊れている。作り直して 1 度だけやり直す。
                    WorkspaceEngine.openE5(db, docs, installModel(context, force = true).absolutePath, e5SearchConfig())
                }
                // 分割方式が変わっていたら、原本から索引を作り直す（普段は何もしない）。
                e.ensureCurrent()
                // 初回だけ、サンプルの資料を登録する（あとで削除した資料を、開くたびに戻さない）。
                val marker = File(context.filesDir, "workspace-seeded")
                if (!marker.exists()) {
                    SAMPLE_DOCS.forEach { e.addDocument(it.id, it.title, DocFormat.MARKDOWN, it.markdown) }
                    marker.writeText("1")
                }
                e to e.documents()
            }
            runOnUiThread {
                result.onSuccess { (e, list) ->
                    if (isDestroyed) { e.close(); return@onSuccess }
                    engine = e
                    screen.documents(list)
                    drainPending()
                }.onFailure {
                    screen.failed(userMessage(it, debuggable, "検索の準備に失敗しました"))
                    if (pending.isNotEmpty()) screen.note("取り込めなかった資料が ${pending.size} 件あります", ok = false)
                }
            }
        }
    }

    /** 重い処理（索引の更新・検索）を別スレッドで行い、結果を UI スレッドへ戻す。 */
    private fun <T> inBackground(name: String, work: (WorkspaceEngine) -> T, done: (Result<T>) -> Unit) {
        val e = engine ?: return
        thread(name = name) {
            val outcome = runCatching { work(e) }
            runOnUiThread { if (!isDestroyed) done(outcome) }
        }
    }

    private fun runSearch(query: String) {
        screen.searching()
        inBackground("lm-search", { it.search(query, 3u) }) { outcome ->
            outcome.onSuccess { screen.show(query, it) }
                .onFailure { screen.failed(userMessage(it, debuggable, "検索できませんでした")) }
        }
    }

    /** 入力フォームから追加する（Markdown として扱う）。成功したら入力欄を空にする。 */
    private fun addTyped(title: String, text: String) =
        addDocument(Imported(Importer.cleanTitle(title), text, DocFormat.MARKDOWN), clearForm = true)

    private fun addDocument(doc: Imported, clearForm: Boolean = false, then: () -> Unit = {}) {
        screen.busy(true)
        screen.note("追加しています…", ok = true)
        inBackground("lm-add", {
            val id = "doc-${System.currentTimeMillis()}-${sequence.getAndIncrement()}"
            val n = it.addDocument(id, doc.title, doc.format, doc.text)
            n to it.documents()
        }) { outcome ->
            screen.busy(false)
            outcome.onSuccess { (n, list) ->
                screen.documents(list)
                screen.note("「${doc.title}」を追加しました（断片 $n 件）", ok = true)
                if (clearForm) screen.clearForm()
            }.onFailure { screen.note(userMessage(it, debuggable, "追加できませんでした"), ok = false) }
            then()
        }
    }

    /** 準備前に届いた資料を、1 件ずつ取り込む。 */
    private fun drainPending() {
        val next = pending.removeFirstOrNull() ?: return
        addDocument(next, then = ::drainPending)
    }

    /** ファイル選択（システムの画面）を開く。選んだファイルだけが読める。 */
    private fun pickFile() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("text/plain", "text/markdown", "text/x-markdown"))
        @Suppress("DEPRECATION")
        startActivityForResult(intent, REQUEST_PICK)
    }

    @Deprecated("Activity の素の API。AndroidX を使わないサンプルなので、これを使う")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri = data?.data
        if (requestCode == REQUEST_PICK && resultCode == RESULT_OK && uri != null) {
            load { Importer.fromUri(this, uri) }
        }
    }

    /** 共有シートから届いた資料。処理した intent は消す（同じ共有を 2 度取り込まない）。 */
    private fun handleShare(intent: Intent?) {
        if (intent?.action != Intent.ACTION_SEND) return
        setIntent(Intent())
        load { Importer.fromShare(this, intent) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShare(intent)
    }

    /**
     * ファイルや共有の読み込みを別スレッドで行い、取り込む。読み込みは 1 つずつ（読み込み中に届いた別の共有は、
     * 断って知らせる）。準備前なら、上限つきの待ち行列に入れて、準備ができたら取り込む。
     */
    private fun load(read: () -> Imported) {
        if (importing) {
            screen.note("別のファイルを読み込み中です。終わってから、もう一度お試しください", ok = false)
            return
        }
        importing = true
        screen.note("読み込んでいます…", ok = true)
        importer.execute {
            val result = runCatching(read)
            runOnUiThread {
                importing = false
                if (isDestroyed) return@runOnUiThread
                result.onSuccess(::accept)
                    .onFailure { screen.note(userMessage(it, debuggable, "読み込めませんでした"), ok = false) }
            }
        }
    }

    private fun accept(doc: Imported) {
        when {
            engine != null -> addDocument(doc)
            pending.size >= MAX_QUEUED -> screen.note("準備中に受け付けられるのは $MAX_QUEUED 件までです", ok = false)
            else -> {
                pending.addLast(doc)
                screen.note("「${doc.title}」は、準備ができたら追加します", ok = true)
            }
        }
    }

    /** 削除は取り消せないので、確認してから行う。 */
    private fun confirmRemove(id: String, title: String) {
        AlertDialog.Builder(this)
            .setTitle("「$title」を削除しますか？")
            .setMessage("この資料と、検索用のデータが消えます。元に戻せません。")
            .setPositiveButton("削除") { _, _ -> removeDocument(id) }
            .setNegativeButton("キャンセル", null)
            .show()
    }

    private fun removeDocument(id: String) {
        screen.busy(true)
        inBackground("lm-remove", { it.removeDocument(id); it.documents() }) { outcome ->
            screen.busy(false)
            outcome.onSuccess { screen.documents(it); screen.note("削除しました", ok = true) }
                .onFailure { screen.note(userMessage(it, debuggable, "削除できませんでした"), ok = false) }
        }
    }

    override fun onDestroy() {
        // 検索中に閉じると、ネイティブ側のハンドルが使用中になりうるので、先に参照を外してから閉じる。
        val e = engine
        engine = null
        e?.close()
        importer.shutdown()
        super.onDestroy()
    }
}
