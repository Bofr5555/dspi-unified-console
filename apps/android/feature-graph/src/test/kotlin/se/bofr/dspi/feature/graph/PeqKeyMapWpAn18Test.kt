package se.bofr.dspi.feature.graph

import androidx.compose.ui.input.key.Key
import org.junit.Assert.assertEquals
import org.junit.Test

/** WP AN18: the graph's hardware-keyboard table against macOS PeqGraphEditor keyDown. */
class PeqKeyMapWpAn18Test {
    @Test
    fun wpAn18EveryMacKeyHasItsCompoundKey() {
        val table = mapOf(
            Key.Backspace to PeqKey.DELETE, // Mac 51, delete
            Key.Delete to PeqKey.DELETE, // Mac 117, forward delete
            Key.Escape to PeqKey.ESCAPE,
            Key.Tab to PeqKey.TAB,
            Key.DirectionLeft to PeqKey.LEFT,
            Key.DirectionRight to PeqKey.RIGHT,
            Key.DirectionUp to PeqKey.UP,
            Key.DirectionDown to PeqKey.DOWN,
            Key.A to PeqKey.A,
            Key.B to PeqKey.OTHER,
            Key.Spacebar to PeqKey.OTHER
        )
        for ((key, expected) in table) assertEquals("$key", expected, PeqKeyMap.keyOf(key))
    }

    @Test
    fun wpAn18CommandIsCtrlOrMeta() {
        assertEquals(PeqMods.CTRL, PeqKeyMap.modsOf(ctrl = true, meta = false, alt = false, shift = false))
        assertEquals(PeqMods.CTRL, PeqKeyMap.modsOf(ctrl = false, meta = true, alt = false, shift = false))
        assertEquals(PeqMods.ALT or PeqMods.SHIFT, PeqKeyMap.modsOf(ctrl = false, meta = false, alt = true, shift = true))
        assertEquals(PeqMods.NONE, PeqKeyMap.modsOf(ctrl = false, meta = false, alt = false, shift = false))
    }
}
