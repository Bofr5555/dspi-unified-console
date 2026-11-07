package se.bofr.dspi.feature.settings.cs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.long
import se.bofr.dspi.ui.json.obj

/** WP AN10: the remote buttons' records, the learn's replies and what the page says about them. */
class CsIrAn10Test {
    private val snapshot = """
        {"supported":true,"ir":[
         {"noun":1,"action":3,"flags":0,"target":0,"index":0,"protocol":1,"value":0,"step":1,"code":551502015},
         {"noun":0,"action":0,"flags":0,"target":0,"index":0,"protocol":0,"value":0,"step":0,"code":0},
         {"noun":0,"action":0,"flags":0,"target":0,"index":0,"protocol":0,"value":0,"step":0,"code":0}]}
    """.trimIndent()

    private fun status(code: Int) = if (code == 0x1E) "Add an IR receiver before learning a remote button" else ""

    @Test
    fun an10RecordsAndChips() {
        val r = CsIr.records(snapshot)
        assertEquals(3, r.size)
        assertTrue(CsIr.isLearned(r[0]))
        assertFalse(CsIr.isLearned(r[1]))
        assertEquals("NEC 0x20DF40BF", CsIr.chip(r[0]))
        assertEquals(CsIr.NOT_LEARNED, CsIr.chip(r[1]))
        assertEquals(CsIr.NOT_LEARNED, CsIr.chip("not json"))
        assertEquals(emptyList<String>(), CsIr.records("""{"supported":false}"""))
    }

    @Test
    fun an10FirstFreeSkipsLearnedAndDraftedEntries() {
        val r = CsIr.records(snapshot)
        assertEquals(1, CsIr.firstFree(r))
        assertEquals(2, CsIr.firstFree(r, setOf(1)))
        assertNull(CsIr.firstFree(r, setOf(1, 2)))
    }

    @Test
    fun an10ALearnedCodeKeepsWhatTheButtonDrives() {
        val d = CsIr.withCode(CsIr.records(snapshot)[0], 2, 0xDEADBEEFL)!!
        val m = Json.parseObject(d)!!
        assertEquals(2, m.int("protocol"))
        assertEquals(0xDEADBEEFL, m.long("code"))
        assertEquals(1, m.int("noun"))
        assertEquals(3, m.int("action"))
        assertNull(CsIr.withCode("[]", 1, 1))
    }

    @Test
    fun an10Ops() {
        val op = Json.parseObject(CsIr.op(4, """{"protocol":1,"code":5}""")!!)!!
        assertEquals("ir", op["op"])
        assertEquals(4, op.int("index"))
        assertEquals(5L, op.obj("command")!!.long("code"))
        assertNull(CsIr.op(4, "nope"))
        val clear = Json.parseObject(CsIr.clearOp(2))!!
        assertEquals(emptyMap<String, Any?>(), clear.obj("command"))
        assertEquals("learnArm", Json.parseObject(CsIr.ARM)!!["op"])
        assertEquals("learnRead", Json.parseObject(CsIr.READ)!!["op"])
        assertEquals("learnCancel", Json.parseObject(CsIr.CANCEL)!!["op"])
    }

    @Test
    fun an10LearnRepliesAndOutcomes() {
        val done = CsIr.parseLearn("""{"ok":true,"state":2,"protocol":3,"code":4660}""", ::status).value!!
        assertEquals(CsLearnResult(2, 3, 4660), done)
        assertEquals("Learned a RC6 code. Apply to keep it." to false, CsIr.outcome(done))
        assertEquals(CsIr.LEARN_TIMED_OUT to true, CsIr.outcome(CsLearnResult(3, 0, 0)))
        assertEquals(CsIr.LEARN_TIMED_OUT to true, CsIr.outcome(null))
        // Done without a code, or back to idle (another client cancelled): stopped, not learned
        assertEquals(CsIr.LEARN_STOPPED to false, CsIr.outcome(CsLearnResult(2, 1, 0)))
        assertEquals(CsIr.LEARN_STOPPED to false, CsIr.outcome(CsLearnResult(0, 0, 0)))
        // A refusal is the core's words: its error, else the status in words
        assertEquals("no device", CsIr.parseLearn("""{"ok":false,"error":"no device"}""", ::status).error)
        assertEquals(
            "Add an IR receiver before learning a remote button",
            CsIr.parseLearn("""{"ok":false,"status":30}""", ::status).error
        )
        assertFalse(CsIr.parseLearn("garbage", ::status).ok)
    }

    @Test
    fun an10ProtocolNamesAreTheCores() {
        assertEquals(listOf("None", "NEC", "RC5", "RC6", "Generic", "None"), (0..5).map(CsIr::protocolName))
    }
}
