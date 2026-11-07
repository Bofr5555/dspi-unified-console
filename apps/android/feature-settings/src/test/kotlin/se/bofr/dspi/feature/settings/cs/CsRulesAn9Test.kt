package se.bofr.dspi.feature.settings.cs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** WP AN9: the pages' reading of CG20's replies, and the picker's rules at real firmware length. */
class CsRulesAn9Test {
    private val described = """
        {"ok":true,"title":"Push Button","summary":"Mutes all outputs","rows":[
         {"id":"type","title":"Component","detail":"What is wired to the GPIO.","kind":"pick","value":1,"text":"Push Button",
          "options":[{"value":1,"text":"Push Button"},{"value":2,"text":"Toggle Switch"}]},
         {"id":"gpio0","title":"GPIO","detail":"","kind":"pick","value":4,"text":"GPIO 4",
          "options":[{"value":4,"text":"GPIO 4"},{"value":5,"text":"GPIO 5","owner":"I2S","disabled":true}]},
         {"id":"value","title":"Level","detail":"","kind":"number","value":-6.5,"min":-60,"max":0,"step":0.5,"unit":"dB","decimals":1},
         {"id":"wrap","title":"Wrap","detail":"","kind":"toggle","on":true,"enabled":false},
         {"id":"delayNote","title":"","detail":"Delays need firmware 1.2","kind":"note","warning":true},
         {"id":"future","title":"New","detail":"x","kind":"slider"}]}
    """.trimIndent()

    @Test
    fun an9ParsesEveryRowKind() {
        val d = CsRules.parseDescription(described).value!!
        assertEquals("Push Button", d.title)
        assertEquals("Mutes all outputs", d.summary)
        assertEquals(listOf("type", "gpio0", "value", "wrap", "delayNote", "future"), d.rows.map { it.id })
        val gpio = d.rows[1] as CsRow.Pick
        assertEquals(CsOption(5, "GPIO 5", null, "I2S", true), gpio.options[1])
        val n = d.rows[2] as CsRow.Number
        assertEquals(-6.5, n.value, 0.0)
        assertEquals("dB", n.unit)
        val t = d.rows[3] as CsRow.Toggle
        assertTrue(t.on && !t.enabled)
        assertTrue((d.rows[4] as CsRow.Note).warning)
        // An unknown kind is kept as words, not dropped
        assertTrue(d.rows[5] is CsRow.Note)
    }

    @Test
    fun an9RefusalsCarryTheCoresWords() {
        val r = CsRules.parseDescription("""{"ok":false,"error":"binding 40 does not exist on this device"}""")
        assertFalse(r.ok)
        assertEquals("binding 40 does not exist on this device", r.error)
        assertFalse(CsRules.parseDescription("garbage").ok)
        assertEquals("noun does not offer 9999", CsRules.parseDraft("""{"ok":false,"error":"noun does not offer 9999"}""").error)
        assertEquals("""{"type":1,"gpio0":4}""", CsRules.parseDraft("""{"ok":true,"draft":{"type":1,"gpio0":4}}""").value)
    }

    @Test
    fun an9ApplyErrorPrefersErrorTextThenStatus() {
        assertNull(CsRules.applyError("""{"ok":true,"status":0}""") { "" })
        assertEquals(
            "firmware mismatch: read-only",
            CsRules.applyError("""{"ok":false,"status":255,"error":"firmware mismatch: read-only"}""") {
                "x"
            }
        )
        assertEquals(
            "A pin is already claimed",
            CsRules.applyError("""{"ok":false,"status":2}""") {
                if (it ==
                    2
                ) {
                    "A pin is already claimed"
                } else {
                    ""
                }
            }
        )
        assertEquals("Refused (status 99)", CsRules.applyError("""{"ok":false,"status":99}""") { "" })
    }

    @Test
    fun an9OpsAndChanges() {
        assertEquals("""{"op":"binding","index":3,"binding":{"type":1,"gpio0":4}}""", CsRules.bindingOp(3, """{"type":1,"gpio0":4}"""))
        assertNull(CsRules.bindingOp(3, "[1]"))
        assertEquals("""{"id":"noun","value":2}""", CsRules.change("noun", 2))
        assertEquals("""{"id":"wrap","value":true}""", CsRules.change("wrap", true))
        assertEquals("""{"id":"value","value":-3.5}""", CsRules.change("value", -3.5))
    }

    @Test
    fun an9NumberEntryKeepsToTheRange() {
        val n = CsRules.parseDescription(described).value!!.rows[2] as CsRow.Number
        assertEquals("-6.5 dB", CsRules.formatNumber(n))
        assertEquals(-12.0, CsRules.parseNumber(n, "-12").value!!, 0.0)
        assertEquals(-3.5, CsRules.parseNumber(n, "-3,5 dB").value!!, 0.0)
        assertEquals("Out of range: -60.0 to 0.0 dB", CsRules.parseNumber(n, "3").error)
        assertEquals("Not a number", CsRules.parseNumber(n, "loud").error)
        // The shown end of the range is inside it
        assertEquals(0.0, CsRules.parseNumber(n, "0.04").value!!, 0.0)
    }

    /** AN8 U1: real firmware offers about 70 functions on a button, in 16 families. */
    private val firmwareFunctions = (0 until 79).map { CsOption(it, "Function $it", "Family ${it / 5}") }

    @Test
    fun an9PickerStyleAndSearchAtFirmwareLength() {
        val d = CsRules.parseDescription(described).value!!
        assertEquals(CsPickStyle.MENU, CsRules.pickStyle(d.rows[0] as CsRow.Pick))
        // Owners and disabled pins need the page, however short
        assertEquals(CsPickStyle.PAGE, CsRules.pickStyle(d.rows[1] as CsRow.Pick))
        val noun = CsRow.Pick("noun", "Controls", "", 0, "Function 0", firmwareFunctions)
        assertEquals(CsPickStyle.PAGE, CsRules.pickStyle(noun))
        assertTrue(noun.options.size > CsRules.SEARCH_MIN)
        assertEquals(16, CsRules.sections(firmwareFunctions).size)
        assertEquals(listOf(7, 70, 71, 72, 73, 74, 75, 76, 77, 78), CsRules.filter(firmwareFunctions, "function 7").map { it.value })
        assertEquals((10..14).toList(), CsRules.filter(firmwareFunctions, "FAMILY 2").map { it.value })
        assertEquals(79, CsRules.filter(firmwareFunctions, "  ").size)
        assertTrue(CsRules.filter(firmwareFunctions, "nothing").isEmpty())
    }

    @Test
    fun an9RoutesSurviveSaving() {
        for (r in listOf(CsRoute.List, CsRoute.Add, CsRoute.Edit(7))) assertEquals(r, CsRoute.restore(CsRoute.save(r)))
        assertEquals(CsRoute.List, CsRoute.restore("edit/x"))
    }
}
