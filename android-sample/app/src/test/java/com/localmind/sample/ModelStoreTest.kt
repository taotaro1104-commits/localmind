package com.localmind.sample

import java.io.File
import java.nio.file.Files
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

class ModelStoreTest {
    private lateinit var dir: File
    private val shaA = "a".repeat(64)
    private val shaB = "b".repeat(64)

    @Before fun setUp() { dir = Files.createTempDirectory("lm-models").toFile() }
    @After fun tearDown() { dir.deleteRecursively() }

    private fun model(name: String) = File(dir, name).also { it.writeText("x") }

    @Test
    fun namesMustBeAPlainGgufFileName() {
        for (ok in listOf("a.gguf", "custom-0123456789ab.gguf", "qwen2.5-1.5b-instruct-q4_k_m.gguf")) assertTrue(ok, ModelStore.isValidName(ok))
        for (bad in listOf("../a.gguf", "a/b.gguf", "a\b.gguf", "..gguf", "a.bin", "", "a b.gguf", "x".repeat(101) + ".gguf")) {
            assertFalse(bad, ModelStore.isValidName(bad))
        }
    }

    @Test
    fun shaMustBeLowercaseHex64() {
        assertTrue(ModelStore.isValidSha(shaA))
        for (bad in listOf("", "A".repeat(64), "a".repeat(63), "g".repeat(64))) assertFalse(bad, ModelStore.isValidSha(bad))
    }

    @Test
    fun noRecordMeansNoModelUnlessTheLegacyStandardFileExists() {
        assertNull(ModelStore.active(dir))
        val legacy = model(MODEL_CATALOG[0].fileName)
        assertEquals(legacy, ModelStore.active(dir)?.file)
        assertEquals(MODEL_CATALOG[0].sha256, ModelStore.active(dir)?.sha256)
    }

    @Test
    fun damagedRecordsAreIgnored() {
        model("a.gguf")
        for (bad in listOf("../a.gguf|$shaA", "a.gguf|xyz", "a.gguf", "|", "missing.gguf|$shaA", "a".repeat(600))) {
            File(dir, "active.txt").writeText(bad)
            assertNull(bad, ModelStore.active(dir))
        }
    }

    @Test
    fun activatingSwitchesTheModelButKeepsTheOldFileUntilPruned() {
        val a = model("a.gguf"); val b = model("b.gguf"); val part = model("dl-0123456789ab.part")
        ModelStore.activate(dir, a, shaA)
        assertEquals(shaA, ModelStore.active(dir)?.sha256)
        ModelStore.activate(dir, b, shaB)
        assertEquals("b.gguf", ModelStore.active(dir)?.file?.name)
        assertTrue("前のモデルは、まだ消さない", a.exists())
        assertFalse("一時ファイルを残さない", File(dir, "active.txt.tmp").exists())

        ModelStore.prune(dir, alsoParts = false) // ダウンロード中は、取りかけを残す
        assertTrue("取得済みのモデルは消さない", a.exists()); assertTrue(part.exists())
        ModelStore.prune(dir, alsoParts = true)
        assertFalse(part.exists()); assertTrue(a.exists()); assertTrue(b.exists())
    }

    @Test
    fun installedModelsAreListedAndCanBeSwitchedTo() {
        val a = model("a.gguf"); val b = model("b.gguf")
        assertEquals(emptyList<String>(), ModelStore.installed(dir).map { it.file.name })
        ModelStore.activate(dir, a, shaA)
        ModelStore.activate(dir, b, shaB)
        assertEquals(listOf("a.gguf", "b.gguf"), ModelStore.installed(dir).map { it.file.name })
        ModelStore.switchTo(dir, "a.gguf")
        assertEquals("a.gguf", ModelStore.active(dir)?.file?.name)
        assertEquals(shaA, ModelStore.active(dir)?.sha256)
        // 記録の無い名前・不正な名前には切り替えない
        for (bad in listOf("zzz.gguf", "../a.gguf", "")) {
            try { ModelStore.switchTo(dir, bad); fail(bad) } catch (_: IllegalStateException) {}
        }
        assertEquals("a.gguf", ModelStore.active(dir)?.file?.name)
    }

    @Test
    fun switchingAwayFromAnUnregisteredActiveModelKeepsItInTheList() {
        val legacy = model(MODEL_CATALOG[0].fileName)
        val b = model("b.gguf")
        ModelStore.activate(dir, b, shaB) // 登録簿が無かった状態で、別のモデルを使い始める
        assertEquals(listOf("b.gguf", legacy.name).sorted(), ModelStore.installed(dir).map { it.file.name }.sorted())
        ModelStore.switchTo(dir, legacy.name)
        assertEquals(MODEL_CATALOG[0].sha256, ModelStore.active(dir)?.sha256)
    }

    @Test
    fun theActiveModelCannotBeRemovedButOthersCanAndTheListFollows() {
        val a = model("a.gguf"); val b = model("b.gguf")
        ModelStore.activate(dir, a, shaA)
        ModelStore.activate(dir, b, shaB)
        try { ModelStore.remove(dir, "b.gguf"); fail("使っているモデルは消せない") } catch (_: IllegalStateException) {}
        assertTrue(b.exists())
        ModelStore.remove(dir, "a.gguf")
        assertFalse(a.exists())
        assertEquals(listOf("b.gguf"), ModelStore.installed(dir).map { it.file.name })
        // 一覧の記録からも消えている（ファイルを置き直しても、勝手には戻らない）
        model("a.gguf")
        assertEquals(listOf("b.gguf"), ModelStore.installed(dir).map { it.file.name })
    }

    @Test
    fun aModelRecordedBeforeTheListExistedStillShowsUpAndPruneNeverDeletesModels() {
        val legacy = model(MODEL_CATALOG[0].fileName)
        val orphan = model("orphan.gguf")
        assertEquals(listOf(legacy.name), ModelStore.installed(dir).map { it.file.name })
        assertEquals(MODEL_CATALOG[0].label, ModelStore.installed(dir)[0].label)
        ModelStore.prune(dir, alsoParts = true)
        assertTrue(legacy.exists()); assertTrue("記録に無い .gguf も、勝手には消さない", orphan.exists())
    }

    @Test
    fun aDamagedListIsIgnoredAndNeverMakesPruneDeleteEverything() {
        val a = model("a.gguf")
        File(dir, "installed.txt").writeText("../x.gguf|$shaA\nnonsense\na.gguf|zz")
        assertEquals(emptyList<String>(), ModelStore.installed(dir).map { it.file.name })
        ModelStore.prune(dir, alsoParts = true)
        assertTrue("記録が読めないときも、モデルは消さない", a.exists())
    }

    @Test(expected = IllegalArgumentException::class)
    fun activatingWithABadRecordIsRefused() {
        ModelStore.activate(dir, File(dir, "a.bin"), shaA)
    }

    @Test
    fun theCatalogIsPinned() {
        for (m in MODEL_CATALOG) {
            assertTrue(m.label, ModelStore.isValidName(m.fileName))
            assertTrue(m.label, ModelStore.isValidSha(m.sha256))
            assertTrue(m.label, m.url.startsWith("https://huggingface.co/"))
            assertTrue(m.label, m.bytes > 0)
        }
        assertEquals(MODEL_CATALOG.size, MODEL_CATALOG.map { it.fileName }.toSet().size)
    }
}
