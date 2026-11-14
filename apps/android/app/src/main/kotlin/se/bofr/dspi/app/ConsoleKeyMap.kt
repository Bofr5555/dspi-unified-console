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

    // -- WP P1AN1 -- the keys Qt's table has (MenuTable.qml, QP18) and the new A/B key
    /** Command+1–9: load preset [slot] (0-based), through the Unsaved Changes flow. */
    data class Preset(val slot: Int) : ConsoleShortcut

    /** Command+S: Commit Parameters…. */
    data object Commit : ConsoleShortcut

    /** Command+Shift+R: Revert to Saved…. */
    data object Revert : ConsoleShortcut

    /** Command+M: mute. */
    data object Mute : ConsoleShortcut

    /** Command+B: bypass the master EQ. */
    data object Bypass : ConsoleShortcut

    /** Command+T: the A/B toggle while a compare runs (PRO1 W4). */
    data object AbToggle : ConsoleShortcut
}

// -- WP P1AN1 --
/** The shortcuts that never act while a text field is being edited (PLAN-CLIENTS 5.9, U3). */
internal val ConsoleShortcut.notWhileEditing: Boolean
    get() = this is ConsoleShortcut.Preset ||
        this == ConsoleShortcut.Commit ||
        this == ConsoleShortcut.Revert ||
        this == ConsoleShortcut.Mute ||
        this == ConsoleShortcut.Bypass ||
        this == ConsoleShortcut.AbToggle

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
        // -- WP P1AN1 --
        if (key == Key.R && shift) return ConsoleShortcut.Revert
        if (!shift) PRESET_KEYS.indexOf(key).takeIf { it >= 0 }?.let { return ConsoleShortcut.Preset(it) }
        if (shift) return commandShift[key]?.let { ConsoleShortcut.Open(it) }
        return when (key) {
            Key.Comma -> ConsoleShortcut.Open(DspiRoutes.SETTINGS)
            Key.C -> ConsoleShortcut.CopyChannel
            Key.V -> ConsoleShortcut.PasteChannel
            // Import / Export Filters: the File menu is on Presets
            Key.I, Key.E -> ConsoleShortcut.Open(DspiRoutes.PRESETS)
            // -- WP P1AN1 --
            Key.S -> ConsoleShortcut.Commit
            Key.M -> ConsoleShortcut.Mute
            Key.B -> ConsoleShortcut.Bypass
            Key.T -> ConsoleShortcut.AbToggle
            else -> null
        }
    }

    // -- WP P1AN1 --
    private val PRESET_KEYS = listOf(Key.One, Key.Two, Key.Three, Key.Four, Key.Five, Key.Six, Key.Seven, Key.Eight, Key.Nine)
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

    // -- WP P1AN1 -- each false when nothing was done, so the key goes on unhandled
    fun loadPreset(slot: Int): Boolean = false

    fun commit(): Boolean = false

    fun revert(): Boolean = false

    fun toggleMute(): Boolean = false

    fun toggleBypass(): Boolean = false

    /** The A/B toggle: only while a compare runs and no safe apply waits (the core refuses it then). */
    fun abToggle(): Boolean = false
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
        // -- WP P1AN1 --
        is ConsoleShortcut.Preset -> host.writesAllowed && host.loadPreset(shortcut.slot)
        ConsoleShortcut.Commit -> host.writesAllowed && host.commit()
        ConsoleShortcut.Revert -> host.writesAllowed && host.revert()
        ConsoleShortcut.Mute -> host.writesAllowed && host.toggleMute()
        ConsoleShortcut.Bypass -> host.writesAllowed && host.toggleBypass()
        ConsoleShortcut.AbToggle -> host.writesAllowed && host.abToggle()
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
