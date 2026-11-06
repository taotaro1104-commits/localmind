package com.localmind.sample

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.lm_ffi.DocFormat
import uniffi.lm_ffi.DocumentInfo

class DocGroupsTest : JapaneseUi() {
    private fun doc(id: String, group: String) = DocumentInfo(id, "題$id", DocFormat.TEXT, group, 1u)

    private val docs = listOf(doc("a", "人事"), doc("b", ""), doc("c", "経理"), doc("d", "人事"), doc("e", "Admin"))

    @Test
    fun sectionsAreSortedByNameWithUngroupedLastAndKeepDocumentOrder() {
        val s = DocGroups.sections(docs)
        assertEquals(listOf("Admin", "人事", "経理", ""), s.map { it.key })
        assertEquals(listOf("a", "d"), s[1].docs.map { it.id })
        assertEquals("未分類", s.last().label)
    }

    @Test
    fun noUngroupedSectionWhenEveryDocumentHasAGroup() {
        assertEquals(listOf("人事"), DocGroups.sections(listOf(doc("a", "人事"))).map { it.key })
        assertEquals(emptyList<DocSection>(), DocGroups.sections(emptyList()))
    }

    @Test
    fun namesExcludeUngrouped() {
        assertEquals(listOf("Admin", "人事", "経理"), DocGroups.names(docs))
    }

    @Test
    fun visibleFiltersToTheSelectedGroupOrShowsAll() {
        val s = DocGroups.sections(docs)
        assertEquals(4, DocGroups.visible(s, null).size)
        assertEquals(listOf("人事"), DocGroups.visible(s, "人事").map { it.key })
        assertEquals(listOf(""), DocGroups.visible(s, "").map { it.key })
        assertEquals(emptyList<DocSection>(), DocGroups.visible(s, "存在しない"))
    }

    @Test
    fun aSelectionThatNoLongerExistsFallsBackToAll() {
        val s = DocGroups.sections(docs)
        assertEquals("人事", DocGroups.keepSelection(s, "人事"))
        assertEquals("", DocGroups.keepSelection(s, ""))
        assertNull(DocGroups.keepSelection(s, "消えたグループ"))
        assertNull(DocGroups.keepSelection(s, null))
    }
}
