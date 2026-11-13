// WP AN35: the app's hardware-keyboard shortcuts, the macOS menu commands
// (DSPi_ConsoleApp.swift:11684-12045) and the generator's Space
// (TestSignalsView.swift:941,948) on Compose keys, as Qt's QD17 table
// (apps/desktop/README.md "Keyboard shortcuts"). Command is Ctrl or Meta here,
// as in AN18's PeqKeyMap. Windows open as routes: a tool page, the analysis
// screen, Presets for the File and AutoEQ menus.
//
// Left to the focused view, not mapped here: Cut, Select All and Delete (text
// fields; the graph's Cmd-A, Delete and arrows are PeqKeyMap's), the
// Interrupt Monitor's Cmd-P/K, and Minimise to Menu Bar (no Android menu bar).
package se.bofr.dspi.app

import androidx.compose.ui.input.key.Key
import se.bofr.dspi.ui.DspiRoutes

/** What a shortcut does. */
internal sealed interface ConsoleShortcut {
    /** Open a window: [route] (macOS `openWindow`). */
    data class Open(val route: String) : ConsoleShortcut

    /** Edit > Copy: the selected channel's parameters (macOS `copyChannelParams`, M45). */
    data object CopyChannel : ConsoleShortcut

    /** Edit > Paste: onto the selected channel (macOS `pasteChannelParams`, M45). */
    data object PasteChannel : ConsoleShortcut

    /** Space: the Signal Generator's Start / Stop. */
    data object StartStopGenerator : ConsoleShortcut

    // -- WP AN42 -- Edit > Undo / Redo (the core's journal)
    /** Command+Z. */
    data object Undo : ConsoleShortcut

    /** Command+Shift+Z, and Command+Y as on the web and Windows. */
    data object Redo : ConsoleShortcut
}

internal object ConsoleKeyMap {
    /** Command+Shift: the Tools menu (DSPi_ConsoleApp.swift:11984-12045) and AutoEQ's Browse Profiles. */
    private val commandShift: Map<Key, String> = mapOf(
        Key.M to DspiRoutes.MATRIX,
        Key.L to DspiRoutes.tool("loudness"),
        Key.X to DspiRoutes.tool("crossfeed"),
        Key.P to DspiRoutes.tool("psybass"),
        Key.S to DspiRoutes.tool("subharm"),
        Key.D to DspiRoutes.tool("tube"),
        Key.U to DspiRoutes.tool("upmix"),
        Key.V to DspiRoutes.tool("leveller"),
        // Signal Generator, Spectrum Analyser, Stats, Interrupt Monitor: pages of one screen here
        Key.G to DspiRoutes.ANALYSIS,
        Key.A to DspiRoutes.ANALYSIS,
        Key.T to DspiRoutes.ANALYSIS,
        Key.I to DspiRoutes.ANALYSIS,
        Key.B to DspiRoutes.PRESETS
    )

    /**
     * The shortcut for [key] with these modifiers, or null for a key the app
     * leaves to the focused view. [command] is Ctrl or Meta.
     */
    fun shortcutOf(key: Key, command: Boolean, shift: Boolean, alt: Boolean): ConsoleShortcut? {
        if (alt) return null
        if (!command) return if (key == Key.Spacebar && !shift) ConsoleShortcut.StartStopGenerator else null
        // -- WP AN42 --
        if (key == Key.Z) return if (shift) ConsoleShortcut.Redo else ConsoleShortcut.Undo
        if (key == Key.Y && !shift) return ConsoleShortcut.Redo
        if (shift) return commandShift[key]?.let { ConsoleShortcut.Open(it) }
        return when (key) {
            Key.Comma -> ConsoleShortcut.Open(DspiRoutes.SETTINGS)
            Key.C -> ConsoleShortcut.CopyChannel
            Key.V -> ConsoleShortcut.PasteChannel
            // Import / Export Filters: the File menu is on Presets
            Key.I, Key.E -> ConsoleShortcut.Open(DspiRoutes.PRESETS)
            else -> null
        }
    }
}

/** What [ConsoleShortcuts.perform] reads and drives; the scaffold's nav and bridge in the app, a fake in tests. */
internal interface ShortcutHost {
    /** The current entry's route with its arguments filled in (`tools/crossfeed`), null before the first. */
    val currentRoute: String?

    /** macOS `activeEqChannel`: the open channel page, null on the overview. */
    val selectedChannel: Int?

    /** G1: the device takes writes. */
    val writesAllowed: Boolean

    fun canPaste(): Boolean

    fun navigate(route: String)

    fun copyChannel(ch: Int)

    fun pasteChannel(ch: Int)

    val generatorRunning: Boolean

    /** The Start button is enabled (`startBlocker` is null). */
    val generatorCanStart: Boolean

    fun startGenerator()

    fun stopGenerator()

    // -- WP AN42 -- a step of the core's journal; false when nothing was sent
    // (nothing to step, read-only, a viewer), so the key goes on unhandled
    fun undo(): Boolean = false

    fun redo(): Boolean = false
}

internal object ConsoleShortcuts {
    /**
     * Does [shortcut] on [host]; false when it does nothing here, so the key
     * goes on unhandled. Space acts only on the analysis screen, where the
     * generator's Start / Stop is (the Mac's key belongs to that button).
     */
    fun perform(shortcut: ConsoleShortcut, host: ShortcutHost): Boolean = when (shortcut) {
        is ConsoleShortcut.Open -> {
            if (host.currentRoute != shortcut.route) host.navigate(shortcut.route)
            true
        }
        ConsoleShortcut.CopyChannel -> host.selectedChannel?.let { host.copyChannel(it) } != null
        ConsoleShortcut.PasteChannel -> {
            val ch = host.selectedChannel
            if (ch != null && host.writesAllowed && host.canPaste()) {
                host.pasteChannel(ch)
                true
            } else {
                false
            }
        }
        ConsoleShortcut.Undo -> host.undo() // -- WP AN42 --
        ConsoleShortcut.Redo -> host.redo()
        ConsoleShortcut.StartStopGenerator -> when {
            host.currentRoute != DspiRoutes.ANALYSIS -> false
            host.generatorRunning -> {
                host.stopGenerator()
                true
            }
            host.generatorCanStart && host.writesAllowed -> {
                host.startGenerator()
                true
            }
            else -> false
        }
    }

    /** [pattern] (`tools/{tool}`) with each `{name}` replaced by [arg]`(name)`. */
    fun resolve(pattern: String, arg: (String) -> String?): String =
        Regex("\\{([^}]+)\\}").replace(pattern) { m -> arg(m.groupValues[1]) ?: m.value }
}
