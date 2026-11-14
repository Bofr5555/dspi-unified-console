package se.bofr.dspi.app

import androidx.compose.ui.input.key.Key
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.ui.DspiRoutes

/** WP P1AN1: the keys Qt's table has that Android lacked (presets 1–9, commit, revert, mute, bypass) and the A/B key. */
class ConsoleKeyMapWpP1an1Test {
    private fun cmd(key: Key, shift: Boolean = false) = ConsoleKeyMap.shortcutOf(key, command = true, shift = shift, alt = false)

    @Test
    fun p1an1TheNewKeys() {
        val digits = listOf(Key.One, Key.Two, Key.Three, Key.Four, Key.Five, Key.Six, Key.Seven, Key.Eight, Key.Nine)
        digits.forEachIndexed { i, k -> assertEquals(ConsoleShortcut.Preset(i), cmd(k)) }
        assertNull("no preset 10", cmd(Key.Zero))
        assertEquals(ConsoleShortcut.Commit, cmd(Key.S))
        assertEquals(ConsoleShortcut.Revert, cmd(Key.R, shift = true))
        assertEquals(ConsoleShortcut.Mute, cmd(Key.M))
        assertEquals(ConsoleShortcut.Bypass, cmd(Key.B))
        // -- WP P1DCFIX -- the A/B key is a plain B now; Command+T is nothing
        assertNull(cmd(Key.T))
        // The Tools menu keeps its Command+Shift keys
        assertEquals(ConsoleShortcut.Open(DspiRoutes.MATRIX), cmd(Key.M, shift = true))
        assertEquals(ConsoleShortcut.Open(DspiRoutes.ANALYSIS), cmd(Key.T, shift = true))
        assertNull("a plain key stays with the view", ConsoleKeyMap.shortcutOf(Key.T, command = false, shift = false, alt = false))
        listOf(ConsoleShortcut.Preset(0), ConsoleShortcut.Commit, ConsoleShortcut.AbToggle).forEach { assertTrue(it.notWhileEditing) }
        assertFalse(ConsoleShortcut.Undo.notWhileEditing)
    }

    private class Host(override val writesAllowed: Boolean) : ShortcutHost {
        val log = mutableListOf<String>()
        override val currentRoute: String? = DspiRoutes.HOME
        override val selectedChannel: Int? = null

        override fun canPaste() = false

        override fun navigate(route: String) = Unit

        override fun copyChannel(ch: Int) = Unit

        override fun pasteChannel(ch: Int) = Unit

        override val generatorRunning = false
        override val generatorCanStart = false

        override fun startGenerator() = Unit

        override fun stopGenerator() = Unit

        override fun loadPreset(slot: Int): Boolean = log.add("preset $slot")

        override fun abToggle(): Boolean = log.add("ab")

        override fun toggleMute(): Boolean = log.add("mute")
    }

    @Test
    fun p1an1TheyActOnlyWhileTheDeviceTakesWrites() {
        val on = Host(writesAllowed = true)
        assertTrue(ConsoleShortcuts.perform(ConsoleShortcut.Preset(2), on))
        assertTrue(ConsoleShortcuts.perform(ConsoleShortcut.AbToggle, on))
        assertTrue(ConsoleShortcuts.perform(ConsoleShortcut.Mute, on))
        assertFalse("not offered here: the key goes on", ConsoleShortcuts.perform(ConsoleShortcut.Commit, on))
        assertEquals(listOf("preset 2", "ab", "mute"), on.log)
        val off = Host(writesAllowed = false)
        assertFalse(ConsoleShortcuts.perform(ConsoleShortcut.AbToggle, off))
        assertFalse(ConsoleShortcuts.perform(ConsoleShortcut.Preset(0), off))
        assertTrue(off.log.isEmpty())
    }
}
