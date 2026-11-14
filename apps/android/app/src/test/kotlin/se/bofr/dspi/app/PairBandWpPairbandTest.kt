package se.bofr.dspi.app

import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HistoryStep
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.FilterParams
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * WP PAIRBAND on the real core's demo device (P1AN2's stack): a linked input
 * pair's band edit from the band list, its bypass and a graph drag (live
 * ticks, then the commit) are one undo step each, and one undo puts both
 * sides back together (the core's `set_input_pair_filter`).
 */
class PairBandWpPairbandTest {
    private class Stack(val bridge: DspiBridge, val core: FfiDspiCore, val main: CoroutineDispatcher)

    private fun withDemo(block: suspend Stack.() -> Unit) = runBlocking {
        val executor = Executors.newSingleThreadExecutor()
        val main = executor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val ffi = FfiDspiCore()
        val session = startDemoSession(parent, "pairband", main = main, core = DemoCore(ffi))
        try {
            withTimeout(15_000) {
                session.bridge.status.first { it.connected }
                session.bridge.writesAllowed.first { it }
            }
            Stack(session.bridge, ffi, main).block()
        } finally {
            withContext(main) { session.close() }
            parent.cancel()
            executor.shutdown()
        }
    }

    private suspend fun until(what: String, ms: Long = 15_000, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + ms
        while (!cond()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            delay(20)
        }
    }

    private suspend fun <T> Stack.onMain(block: suspend () -> T): T = withContext(main) { block() }

    /** Both sides of the pair (app ids 0 and 1), band [b], from the core's own state. */
    private fun Stack.sides(b: Int): Pair<FilterParams, FilterParams> = core.getState().filters.let { it[0][b] to it[1][b] }

    private suspend fun Stack.linkPairZero() {
        onMain { bridge.setInputLinked(0, true, keepCh = 0) }
        until("linked") { bridge.isInputLinked(0) }
        until("matched") { (0 until DspiConstants.BANDS_PER_CHANNEL).all { sides(it).first == sides(it).second } }
    }

    @Test
    fun pairbandABandListEditOfALinkedPairIsOneUndo() = withDemo {
        linkPairZero()
        val before = sides(1)
        onMain { bridge.setFilter(0, 1, DspiConstants.FILTER_PEAKING, 1500f, 4f, 1.2f) }
        until("both written") { sides(1).first.filterType == DspiConstants.FILTER_PEAKING && sides(1).second == sides(1).first }
        assertEquals("EQ band, linked pair", core.history(1).first().label)
        val step = onMain { bridge.undo() }
        assertEquals("EQ band, linked pair", (step as HistoryStep.Moved).move.entry?.label)
        until("one undo, both sides back") { sides(1) == before }

        // The bypass of a linked pair's band: one step too
        onMain { bridge.setFilter(1, 2, DspiConstants.FILTER_PEAKING, 800f, 3f, 1f) }
        until("band") { sides(2).first.filterType == DspiConstants.FILTER_PEAKING && sides(2).second.filterType == DspiConstants.FILTER_PEAKING }
        onMain { bridge.setBandBypass(1, 2, true) }
        until("bypassed") { sides(2).first.bypass && sides(2).second.bypass }
        onMain { bridge.undo() }
        until("one undo, both enabled") { !sides(2).first.bypass && !sides(2).second.bypass }
    }

    @Test
    fun pairbandAGraphDragOfALinkedPairIsOneUndo() = withDemo {
        linkPairZero()
        val start = sides(3)
        val p = { gain: Float, freq: Float -> FilterParams(DspiConstants.FILTER_PEAKING, false, freq, 1f, gain, 0.707f) }
        onMain { bridge.setDragging(true) }
        for (i in 1..5) onMain { bridge.sendBandLive(0, 3, p(i.toFloat(), 1000f)) }
        for (i in 1..5) onMain { bridge.sendBandLive(0, 3, p(5f, 1000f + 100f * i)) }
        onMain { bridge.commitBands(0, listOf(3), listOf(p(5f, 1500f))) }
        onMain { bridge.setDragging(false) }
        until("the drag landed on both sides") { sides(3).first.freq == 1500f && sides(3).second.freq == 1500f }
        onMain { bridge.undo() }
        until("one undo, both sides back to the drag's start") { sides(3) == start }
        assertTrue(core.canRedo())
        onMain { bridge.redo() }
        until("one redo, the whole drag again") { sides(3).first.freq == 1500f && sides(3).second.freq == 1500f }
    }

    @Test
    fun pairbandAPasteOntoALinkedInputIsOneUndo() = withDemo {
        linkPairZero()
        val outApp = 2 // OUT1
        for (b in 0 until 3) onMain { bridge.setFilter(outApp, b, DspiConstants.FILTER_PEAKING, 300f + 700f * b, 1.5f + b, 1f) }
        until("source bands") { (0 until 3).all { bridge.filterType(outApp, it) == DspiConstants.FILTER_PEAKING } }
        val before = (0 until DspiConstants.BANDS_PER_CHANNEL).map { sides(it) }
        onMain { bridge.copyChannel(outApp) }
        onMain { bridge.pasteChannel(0) }
        until("pasted on both sides") { (0 until 3).all { sides(it).first.freq == 300f + 700f * it && sides(it).second.freq == 300f + 700f * it } }
        assertEquals("Paste EQ", core.history(1).first().label)
        val step = onMain { bridge.undo() }
        assertEquals("Paste EQ", (step as HistoryStep.Moved).move.entry?.label)
        until("one undo, every band of both sides back") { (0 until DspiConstants.BANDS_PER_CHANNEL).map { sides(it) } == before }
    }

    // -- WP PAIRBANDF -- (review pairband F1) Bypass All on a linked pair: one step on the real core
    @Test
    fun pairbandfBypassAllOnALinkedPairIsOneUndo() = withDemo {
        linkPairZero()
        for (b in 0 until 3) onMain { bridge.setFilter(0, b, DspiConstants.FILTER_PEAKING, 400f + 300f * b, 2f, 1f) }
        until("bands") { (0 until 3).all { sides(it).first.filterType == DspiConstants.FILTER_PEAKING && sides(it).second == sides(it).first } }
        onMain { bridge.setAllBandsBypass(0, true, false) }
        until("bypassed") { (0 until 3).all { sides(it).first.bypass && sides(it).second.bypass } }
        assertEquals("Bypass All", core.history(1).first().label)
        onMain { bridge.undo() }
        until("one undo enables every band of both sides") { (0 until 3).none { sides(it).first.bypass || sides(it).second.bypass } }
    }

    // -- WP PAIRBANDF -- (review pairband F2) Link Inputs with a kept side: one step on the real core
    @Test
    fun pairbandfLinkInputsWithAKeptSideIsOneUndo() = withDemo {
        onMain { bridge.setInputLinked(0, false, -1) }
        until("unlinked") { !bridge.isInputLinked(0) }
        for (b in 0 until 3) onMain { bridge.setFilter(0, b, DspiConstants.FILTER_PEAKING, 400f + 300f * b, 2f, 1f) }
        until("bands") { (0 until 3).all { sides(it).first.filterType == DspiConstants.FILTER_PEAKING } }
        val before = (0 until DspiConstants.BANDS_PER_CHANNEL).map { sides(it).second }
        onMain { bridge.setInputLinked(0, true, keepCh = 0) }
        until("copied") { (0 until 3).all { sides(it).second == sides(it).first } }
        assertEquals("Link Inputs", core.history(1).first().label)
        onMain { bridge.undo() }
        until("one undo puts the copied side back") { (0 until DspiConstants.BANDS_PER_CHANNEL).map { sides(it).second } == before }
    }
}

