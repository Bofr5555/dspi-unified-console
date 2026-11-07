package se.bofr.dspi.feature.settings.cs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.obj
import se.bofr.dspi.ui.json.str

/** WP AN11: the display's part of the snapshot, its operations and the panel-state words. */
class CsDisplayAn11Test {
    private val snapshot = """
        {"supported":true,"display":{"maxPages":4,"modelCount":9,
         "config":{"mode":1,"homePage":0,"dwell":30,"overlayHold":20,"brightness":0,"flags":2,"editTimeout":100},
         "pages":[{"noun":0,"target":0,"index":0,"flags":5},{"noun":6,"target":0,"index":0,"flags":1},
                  {"noun":0,"target":0,"index":0,"flags":0},{"noun":0,"target":0,"index":0,"flags":0},
                  {"noun":0,"target":0,"index":0,"flags":0}],
         "status":{"init":2,"page":1,"flags":0,"model":3,"naks":0}}}
    """.trimIndent()

    @Test
    fun an11ParsesPagesConfigAndStatus() {
        val d = CsDisplay.parse(snapshot)
        assertEquals(4, d.maxPages)
        // Only maxPages of the snapshot's (at least 16) records are this device's
        assertEquals(4, d.pages.size)
        assertEquals(listOf(0, 1), d.activePages)
        assertEquals(2, d.firstFreePage)
        assertEquals(1, Json.parseObject(d.config)?.int("mode"))
        assertEquals(CsDisplayStatus(init = 2, page = 1, flags = 0, model = 3, naks = 0), d.status)
    }

    @Test
    fun an11NoDisplayOrGarbageGivesNoPages() {
        for (s in listOf("", "{", """{"supported":true}""", """{"display":{"maxPages":-3}}""")) {
            val d = CsDisplay.parse(s)
            assertEquals(0, d.maxPages)
            assertTrue(d.pages.isEmpty())
            assertNull(d.firstFreePage)
        }
        // Fewer records than pages: the rest are empty slots
        val short = CsDisplay.parse("""{"display":{"maxPages":3,"pages":[{"flags":1}]}}""")
        assertEquals(listOf(0), short.activePages)
        assertEquals(1, short.firstFreePage)
        assertEquals(CsDisplay.PAGE_NONE, short.status.page)
    }

    @Test
    fun an11EveryPageInUseLeavesNoFreeSlot() {
        val full = CsDisplay.parse("""{"display":{"maxPages":2,"pages":[{"flags":1},{"flags":3}]}}""")
        assertNull(full.firstFreePage)
    }

    @Test
    fun an11Operations() {
        val page = Json.parseObject(CsDisplay.pageOp(3, """{"noun":6,"target":0,"index":0,"flags":1}""")!!)!!
        assertEquals("displayPage", page.str("op"))
        assertEquals(3, page.int("index"))
        assertEquals(6, page.obj("page")?.int("noun"))
        val cfg = Json.parseObject(CsDisplay.configOp("""{"mode":2}""")!!)!!
        assertEquals("displayConfig", cfg.str("op"))
        assertEquals(2, cfg.obj("config")?.int("mode"))
        val clear = Json.parseObject(CsDisplay.clearPageOp(5))!!
        assertEquals(5, clear.int("index"))
        assertFalse(CsDisplay.pageActive(Json.write(clear.obj("page"))))
        assertEquals(CsDisplay.pageOp(1, "{}"), CsDisplay.op(CsDisplay.KIND_PAGE, 1, "{}"))
        assertEquals(CsDisplay.configOp("{}"), CsDisplay.op(CsDisplay.KIND_DISPLAY, 7, "{}"))
        assertNull(CsDisplay.op(CsRules.KIND_BINDING, 0, "{}"))
        assertNull(CsDisplay.pageOp(0, "not json"))
        assertEquals("displayStatus", Json.parseObject(CsDisplay.STATUS_READ)?.str("op"))
    }

    @Test
    fun an11PanelStateWords() {
        assertEquals("Running" to false, CsDisplay.stateText(CsDisplayStatus(init = CsDisplay.STATE_LIVE)))
        assertEquals("Starting up" to false, CsDisplay.stateText(CsDisplayStatus(init = CsDisplay.STATE_INIT)))
        assertEquals("Not responding" to true, CsDisplay.stateText(CsDisplayStatus(init = CsDisplay.STATE_ERROR)))
        assertEquals("Not started" to false, CsDisplay.stateText(CsDisplayStatus(init = CsDisplay.STATE_DOWN)))
        assertEquals("Reported by the device.", CsDisplay.stateDetail(CsDisplayStatus()))
        assertTrue(CsDisplay.stateDetail(CsDisplayStatus(naks = 1)).startsWith("1 I2C error so far."))
        assertTrue(CsDisplay.stateDetail(CsDisplayStatus(naks = 300)).startsWith("300 I2C errors so far."))
    }
}
