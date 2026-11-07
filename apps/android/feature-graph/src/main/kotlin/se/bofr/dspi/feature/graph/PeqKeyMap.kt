// WP AN18: the graph's hardware-keyboard map, macOS PeqGraphEditor keyDown
// (key codes 51/117 delete, 53 escape, 48 tab, 123-126 arrows, Cmd-A) on
// Compose keys. Command is Ctrl or Meta here; the editor's keyDown does the rest.
package se.bofr.dspi.feature.graph

import androidx.compose.ui.input.key.Key

object PeqKeyMap {
    /** The editor's key for a Compose [key]; [PeqKey.OTHER] for anything it does not use. */
    fun keyOf(key: Key): PeqKey = when (key) {
        Key.Delete, Key.Backspace -> PeqKey.DELETE
        Key.Escape -> PeqKey.ESCAPE
        Key.Tab -> PeqKey.TAB
        Key.DirectionLeft -> PeqKey.LEFT
        Key.DirectionRight -> PeqKey.RIGHT
        Key.DirectionUp -> PeqKey.UP
        Key.DirectionDown -> PeqKey.DOWN
        Key.A -> PeqKey.A
        else -> PeqKey.OTHER
    }

    /** [PeqMods] bits: Ctrl or Meta is the Mac's Command. */
    fun modsOf(ctrl: Boolean, meta: Boolean, alt: Boolean, shift: Boolean): Int {
        var m = PeqMods.NONE
        if (ctrl || meta) m = m or PeqMods.CTRL
        if (alt) m = m or PeqMods.ALT
        if (shift) m = m or PeqMods.SHIFT
        return m
    }
}
