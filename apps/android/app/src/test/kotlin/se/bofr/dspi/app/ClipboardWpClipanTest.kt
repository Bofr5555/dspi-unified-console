package se.bofr.dspi.app

import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HistoryStep
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.ffi.FfiDspiCore

/**
 * WP CLIPAN on the real core's demo device (P1AN2's stack): Copy / Paste
 * Parameters are the core's copy_channel / paste_channel. A paste onto an
 * output is one "Paste Parameters" undo step that puts every value back; with
 * the installer's lock on, the paste is refused before anything is written and
 * the notice carries the core's words.
 */
class ClipboardWpClipanTest {
    private class Stack(val bridge: DspiBridge, val core: FfiDspiCore, val main: CoroutineDispatcher, val scope: CoroutineScope)

    private fun withDemo(block: suspend Stack.() -> Unit) = runBlocking {
        val executor = Executors.newSingleThreadExecutor()
        val main = executor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val ffi = FfiDspiCore()
        val session = startDemoSession(parent, "clipan", main = main, core = DemoCore(ffi))
        try {
            withTimeout(15_000) {
                session.bridge.status.first { it.connected }
                session.bridge.writesAllowed.first { it }
            }
            Stack(session.bridge, ffi, main, parent).block()
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

    private suspend fun Stack.refusals(): MutableList<BridgeEvent.CommandFailed> {
        val out = java.util.Collections.synchronizedList(mutableListOf<BridgeEvent.CommandFailed>())
        scope.launch { bridge.events.collect { if (it is BridgeEvent.CommandFailed) out += it } }
        delay(50)
        return out
    }

    private val src = 2 // app id of OUT1
    private val dst = 3 // app id of OUT2

    /** Output [out]'s wire channel ([ch] its app id): PEQ, crossover, gain, delay, mute, from the core's own state. */
    private fun Stack.output(ch: Int, out: Int): List<Any> = core.getState().let { s ->
        val w = s.numInputChannels + out
        check(ch == out + 2)
        listOf(
            s.filters[w].take(DspiConstants.BANDS_PER_CHANNEL),
            s.xover[w],
            s.outputGainDb[out],
            s.outputDelayMs[out],
            s.outputMuted[out]
        )
    }

    private suspend fun Stack.shapeSource() {
        for (b in 0 until 3) onMain { bridge.setFilter(src, b, DspiConstants.FILTER_PEAKING, 300f + 700f * b, 1.5f + b, 1f) }
        onMain { bridge.setCrossover(src, 0, DspiConstants.FILTER_HIGHPASS, 90f) }
        onMain { bridge.setOutputGain(0, -6f) }
        onMain { bridge.setOutputDelay(0, 2.5f) }
        onMain { bridge.setOutputMute(0, true) }
        until("source shaped") {
            bridge.outputMuted(0) &&
                bridge.outputDelayMS(0) == 2.5f &&
                bridge.filterType(src, 2) == DspiConstants.FILTER_PEAKING
        }
        onMain { bridge.copyChannel(src) }
        until("copied") { bridge.canPaste() }
    }

    @Test
    fun clipanOneUndoRestoresAPastedOutputOnTheRealCore() = withDemo {
        shapeSource()
        val before = output(dst, 1)
        onMain { bridge.pasteChannel(dst) }
        until("pasted") { output(dst, 1) == output(src, 0) }
        assertEquals("Paste Parameters", core.history(1).first().label)
        val step = onMain { bridge.undo() }
        assertEquals("Paste Parameters", (step as HistoryStep.Moved).move.entry?.label)
        until("one undo, the whole output back") { output(dst, 1) == before }
    }

    @Test
    fun clipanALockedPasteWritesNothingAndShowsTheCoresWords() = withDemo {
        shapeSource()
        onMain { bridge.installer.setPin("4321") }.getOrThrow()
        onMain { bridge.installer.cover(listOf("front")) }.getOrThrow()
        onMain { bridge.installer.lockNow() }.getOrThrow()
        until("locked") { bridge.installer.lock.value?.locked == true }
        val before = output(dst, 1)
        val history = core.history(1)
        val refused = refusals()
        onMain { bridge.pasteChannel(dst) }
        until("refused") { refused.any { it.command == "pasteChannel" } }
        val e = refused.first { it.command == "pasteChannel" }
        assertTrue("${e.error}", e.error is DspiException.InstallerLock)
        val words = coreRefusalText(bridge, e)
        assertTrue(words, words!!.startsWith("Locked by the installer"))
        assertEquals("nothing written", before, output(dst, 1))
        assertEquals("no undo step", history, core.history(1))
    }
}
