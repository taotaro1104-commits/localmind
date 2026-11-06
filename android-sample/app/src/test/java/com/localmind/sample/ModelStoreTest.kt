package com.localmind.sample

import java.io.File
import java.nio.file.Files
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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
        assertFalse(a.exists()); assertTrue(part.exists())
        ModelStore.prune(dir, alsoParts = true)
        assertFalse(part.exists()); assertTrue(b.exists())
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
