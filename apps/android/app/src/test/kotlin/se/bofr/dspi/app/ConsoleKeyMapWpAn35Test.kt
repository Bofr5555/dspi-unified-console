package se.bofr.dspi.app

import androidx.compose.ui.input.key.Key
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.feature.tools.ToolRoute
import se.bofr.dspi.ui.DspiRoutes

/** WP AN35: every macOS shortcut (DSPi_ConsoleApp.swift:11684-12045, TestSignalsView.swift:941) on Android's keys. */
class ConsoleKeyMapWpAn35Test {
    private data class Row(val mac: String, val key: Key, val shift: Boolean, val expect: ConsoleShortcut?)

    private fun open(route: String) = ConsoleShortcut.Open(route)

    /** The Mac's menu, in its order; null: left to the focused view, or no Android equivalent. */
    private val table = listOf(
        Row("Cmd-, Settings", Key.Comma, false, open(DspiRoutes.SETTINGS)),
        Row("Cmd-X Cut (text fields)", Key.X, false, null),
        Row("Cmd-C Copy channel", Key.C, false, ConsoleShortcut.CopyChannel),
        Row("Cmd-V Paste channel", Key.V, false, ConsoleShortcut.PasteChannel),
        Row("Cmd-A Select All (text fields, graph)", Key.A, false, null),
        Row("Cmd-I Import Filters", Key.I, false, open(DspiRoutes.PRESETS)),
        Row("Cmd-E Export Filters", Key.E, false, open(DspiRoutes.PRESETS)),
        Row("Cmd-Shift-B Browse Profiles", Key.B, true, open(DspiRoutes.PRESETS)),
        Row("Cmd-Shift-M Matrix Mixer", Key.M, true, open(DspiRoutes.MATRIX)),
        Row("Cmd-Shift-L Loudness", Key.L, true, open(DspiRoutes.tool("loudness"))),
        Row("Cmd-Shift-X Crossfeed", Key.X, true, open(DspiRoutes.tool("crossfeed"))),
        Row("Cmd-Shift-P Psychoacoustic Bass", Key.P, true, open(DspiRoutes.tool("psybass"))),
        Row("Cmd-Shift-S Subharmonic Synthesizer", Key.S, true, open(DspiRoutes.tool("subharm"))),
        Row("Cmd-Shift-D Tube Modeller", Key.D, true, open(DspiRoutes.tool("tube"))),
        Row("Cmd-Shift-U Stereo Upmixer", Key.U, true, open(DspiRoutes.tool("upmix"))),
        Row("Cmd-Shift-V Volume Leveller", Key.V, true, open(DspiRoutes.tool("leveller"))),
        Row("Cmd-Shift-G Signal Generator", Key.G, true, open(DspiRoutes.ANALYSIS)),
        Row("Cmd-Shift-A Spectrum Analyser", Key.A, true, open(DspiRoutes.ANALYSIS)),
        Row("Cmd-Shift-T Stats", Key.T, true, open(DspiRoutes.ANALYSIS)),
        Row("Cmd-Shift-I Interrupt Monitor", Key.I, true, open(DspiRoutes.ANALYSIS)),
        Row("Cmd-P Interrupt Monitor pause (its page)", Key.P, false, null),
        Row("Cmd-K Interrupt Monitor clear (its page)", Key.K, false, null)
    )

    @Test
    fun wpAn35EveryMacShortcutHasItsKey() {
        for (r in table) {
            // command: Ctrl or Meta (ShortcutKeys; the key-event test below presses Ctrl)
            assertEquals(r.mac, r.expect, ConsoleKeyMap.shortcutOf(r.key, command = true, shift = r.shift, alt = false))
            // Without Command it is typing
            if (r.key != Key.Spacebar) assertNull(r.mac, ConsoleKeyMap.shortcutOf(r.key, command = false, shift = r.shift, alt = false))
        }
        // Every tool page the Tools screen has is reachable
        val toolRoutes = ToolRoute.entries.map { it.route }.toSet()
        val mapped = table.mapNotNull { (it.expect as? ConsoleShortcut.Open)?.route }.filter { it.startsWith("tools/") }.toSet()
        assertEquals(toolRoutes, mapped)
    }

    @Test
    fun wpAn35SpaceAndTheOtherModifiers() {
        assertEquals(
            ConsoleShortcut.StartStopGenerator,
            ConsoleKeyMap.shortcutOf(Key.Spacebar, command = false, shift = false, alt = false)
        )
        assertNull(ConsoleKeyMap.shortcutOf(Key.Spacebar, command = false, shift = true, alt = false))
        assertNull(ConsoleKeyMap.shortcutOf(Key.Spacebar, command = true, shift = false, alt = false))
        // Option-Cmd-M (Minimise to Menu Bar) has no Android menu bar; no Option shortcut is mapped
        assertNull(ConsoleKeyMap.shortcutOf(Key.M, command = true, shift = false, alt = true))
        assertNull(ConsoleKeyMap.shortcutOf(Key.Comma, command = true, shift = false, alt = true))
        assertNull(ConsoleKeyMap.shortcutOf(Key.Delete, command = false, shift = false, alt = false))
    }

    private class FakeHost(
        override var currentRoute: String? = DspiRoutes.HOME,
        override var selectedChannel: Int? = null,
        override var writesAllowed: Boolean = true,
        var clip: Boolean = false,
        override var generatorRunning: Boolean = false,
        override var generatorCanStart: Boolean = true
    ) : ShortcutHost {
        val log = mutableListOf<String>()

        override fun canPaste() = clip

        override fun navigate(route: String) {
            log += "go $route"
            currentRoute = route
        }

        override fun copyChannel(ch: Int) {
            log += "copy $ch"
            clip = true
        }

        override fun pasteChannel(ch: Int) {
            log += "paste $ch"
        }

        override fun startGenerator() {
            log += "start"
        }

        override fun stopGenerator() {
            log += "stop"
        }
    }

    @Test
    fun wpAn35OpenGoesOnceAndCopyPasteNeedAChannel() {
        val h = FakeHost()
        assertTrue(ConsoleShortcuts.perform(open(DspiRoutes.MATRIX), h))
        assertTrue("already there: handled, no second page", ConsoleShortcuts.perform(open(DspiRoutes.MATRIX), h))
        assertEquals(listOf("go matrix"), h.log)

        // The overview has no channel: Copy and Paste pass the key on (macOS: no activeEqChannel)
        assertFalse(ConsoleShortcuts.perform(ConsoleShortcut.CopyChannel, h))
        h.selectedChannel = 3
        assertFalse("nothing to paste yet", ConsoleShortcuts.perform(ConsoleShortcut.PasteChannel, h))
        assertTrue(ConsoleShortcuts.perform(ConsoleShortcut.CopyChannel, h))
        h.selectedChannel = 4
        h.writesAllowed = false
        assertFalse("G1: read-only refuses Paste", ConsoleShortcuts.perform(ConsoleShortcut.PasteChannel, h))
        h.writesAllowed = true
        assertTrue(ConsoleShortcuts.perform(ConsoleShortcut.PasteChannel, h))
        assertEquals(listOf("go matrix", "copy 3", "paste 4"), h.log)
    }

    @Test
    fun wpAn35SpaceIsTheGeneratorsStartStopOnItsScreenOnly() {
        val h = FakeHost(currentRoute = DspiRoutes.HOME)
        assertFalse("elsewhere Space does nothing", ConsoleShortcuts.perform(ConsoleShortcut.StartStopGenerator, h))
        h.currentRoute = DspiRoutes.ANALYSIS
        h.generatorCanStart = false
        assertFalse("Start disabled (startBlocker): no start", ConsoleShortcuts.perform(ConsoleShortcut.StartStopGenerator, h))
        h.generatorCanStart = true
        h.writesAllowed = false
        assertFalse("read-only: Start is disabled", ConsoleShortcuts.perform(ConsoleShortcut.StartStopGenerator, h))
        h.writesAllowed = true
        assertTrue(ConsoleShortcuts.perform(ConsoleShortcut.StartStopGenerator, h))
        h.generatorRunning = true
        h.writesAllowed = false
        assertTrue("Stop always works", ConsoleShortcuts.perform(ConsoleShortcut.StartStopGenerator, h))
        assertEquals(listOf("start", "stop"), h.log)
    }

    @Test
    fun wpAn35RoutesResolveTheirArguments() {
        assertEquals("tools/crossfeed", ConsoleShortcuts.resolve(DspiRoutes.TOOL) { if (it == "tool") "crossfeed" else null })
        assertEquals("channel/3", ConsoleShortcuts.resolve(DspiRoutes.CHANNEL) { "3" })
        assertEquals(DspiRoutes.HOME, ConsoleShortcuts.resolve(DspiRoutes.HOME) { null })
    }
}
