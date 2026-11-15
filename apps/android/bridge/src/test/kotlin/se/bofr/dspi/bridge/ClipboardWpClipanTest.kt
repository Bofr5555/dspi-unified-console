package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.fake.demoDspState

/**
 * WP CLIPAN: Copy / Paste Parameters go through the core's copy_channel and
 * paste_channel (CLIP1), here over the fake's model of them. One undo puts a
 * pasted output back; a paste refused (read-only, or by the core) writes nothing
 * and reports through CommandFailed, the notice path, with the core's words.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ClipboardWpClipanTest {
    private val src = 2 // app id of output 1 (wire 8 on the demo)
    private val dst = 3 // app id of output 2

    /** What the paste touches on [ch] (output [out]). */
    private fun outputView(b: DspiBridgeImpl, ch: Int, out: Int) =
        listOf(b.channelBands(ch), b.outputGainDB(out), b.outputDelayMS(out), b.outputMuted(out))

    private fun TestScope.shapeSource(b: DspiBridgeImpl) {
        for (band in 0 until 3) b.setFilter(src, band, DspiConstants.FILTER_PEAKING, 300f + 500f * band, -2f + band, 1.2f)
        b.setCrossover(src, 0, DspiConstants.FILTER_HIGHPASS, 120f)
        b.setOutputGain(0, -4.5f)
        b.setOutputDelay(0, 3.25f)
        b.setOutputMute(0, true)
        runCurrent()
    }

    @Test
    fun clipanOneUndoRestoresAPastedOutput() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        shapeSource(b)
        b.setOutputGain(1, -1f)
        runCurrent()
        val before = outputView(b, dst, 1)
        assertFalse(b.canPaste())
        b.copyChannel(src)
        runCurrent()
        assertTrue(b.canPaste())
        core.calls.clear()
        b.pasteChannel(dst)
        runCurrent()
        assertEquals(1, core.callCount("pasteChannel"))
        assertEquals(outputView(b, src, 0), outputView(b, dst, 1))
        assertNotEquals(DspiConstants.FILTER_FLAT, b.crossoverType(src, 0))
        assertEquals(b.crossoverType(src, 0), b.crossoverType(dst, 0))
        assertEquals("the core's label", "Paste Parameters", core.history(1).first().label)
        b.undo()
        assertEquals("one undo puts the whole output back", before, outputView(b, dst, 1))
    }

    @Test
    fun clipanAReadOnlyPasteWritesNothing() = runTest {
        val core = FakeDspiCore(demoDspState().copy(compat = DspiConstants.COMPAT_FIRMWARE_TOO_OLD))
        val b = newBridge(core)
        attach(b, core)
        assertFalse(b.writesAllowed.value)
        b.copyChannel(src) // a read: allowed
        runCurrent()
        assertTrue(b.canPaste())
        val before = core.getState()
        val events = collectEvents(b)
        core.calls.clear()
        b.pasteChannel(dst)
        runCurrent()
        val e = events.filterIsInstance<BridgeEvent.CommandFailed>().single()
        assertEquals("pasteChannel", e.command)
        assertTrue(e.error is WritesNotAllowedException)
        assertEquals(0, core.callCount("pasteChannel"))
        assertEquals(before, core.getState())
    }

    @Test
    fun clipanACoreRefusalWritesNothingAndCarriesTheCoresWords() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        shapeSource(b)
        b.copyChannel(src)
        runCurrent()
        val before = core.getState()
        val events = collectEvents(b)
        val refusal = DspiException.InstallerLock("locked", "Locked by the installer: this console is locked")
        core.failNext("pasteChannel") { refusal }
        core.calls.clear()
        b.pasteChannel(dst)
        runCurrent()
        val e = events.filterIsInstance<BridgeEvent.CommandFailed>().single()
        assertEquals("pasteChannel", e.command)
        assertSame(refusal, e.error)
        assertEquals(1, core.callCount("pasteChannel"))
        assertEquals("no setter ran", emptyList<String>(), core.calls.filter { it.startsWith("set") })
        assertEquals(before, core.getState())
        assertNotEquals(outputView(b, src, 0), outputView(b, dst, 1))
    }

    @Test
    fun clipanAPasteOntoALinkedInputNamesThePartner() = runTest {
        val core = FakeDspiCore()
        val b = newBridge(core)
        attach(b, core)
        b.setInputLinked(0, true, keepCh = 0)
        shapeSource(b)
        b.copyChannel(src)
        b.pasteChannel(0)
        runCurrent()
        assertTrue(core.calls.any { it.startsWith("pasteChannel(") && it.endsWith(", 0, 1)") })
        assertEquals(b.channelBands(src)!!.peq, b.channelBands(0)!!.peq)
        assertEquals(b.channelBands(src)!!.peq, b.channelBands(1)!!.peq)
        // an input takes the PEQ only: output 1's strip stays where it was
        assertEquals(-4.5f, b.outputGainDB(0))
    }
}
