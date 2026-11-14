package se.bofr.dspi.app

import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NamedNavArgument
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.analysis.AnalysisScreen
import se.bofr.dspi.feature.analysis.TestSignalsUiState
import se.bofr.dspi.feature.analysis.isRunning
import se.bofr.dspi.feature.firmware.FirmwareUpdateView
import se.bofr.dspi.feature.graph.GraphScreen
import se.bofr.dspi.feature.home.ChannelScreen
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.feature.home.HostChannelsFeed
import se.bofr.dspi.feature.home.system.SystemScreen
import se.bofr.dspi.feature.matrix.MatrixMixerView
import se.bofr.dspi.feature.onboarding.OnboardingSource
import se.bofr.dspi.feature.presets.PRESET_SLOT_COUNT
import se.bofr.dspi.feature.presets.PresetMenuActions
import se.bofr.dspi.feature.presets.PresetsScreen
import se.bofr.dspi.feature.presets.ToolsMenuActions
import se.bofr.dspi.feature.presets.snapshots.SafeApplyBar
import se.bofr.dspi.feature.presets.snapshots.SnapshotsScreen
import se.bofr.dspi.feature.settings.SettingsScreen
import se.bofr.dspi.feature.tools.OutputLimiterView
import se.bofr.dspi.feature.tools.ToolsScreen
import se.bofr.dspi.ui.DragMarker
import se.bofr.dspi.ui.DspiIcons
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.HelpTip
import se.bofr.dspi.ui.LocalDemoMode
import se.bofr.dspi.ui.LocalDragMarker
import se.bofr.dspi.ui.LocalWritesAllowed
import se.bofr.dspi.ui.WhileVisible
import se.bofr.dspi.ui.WritesExempt
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.ConsoleAlertHost
import se.bofr.dspi.ui.link.LinkBanner
import se.bofr.dspi.ui.link.LinkBannerAction
import se.bofr.dspi.ui.state.AppState
import se.bofr.dspi.ui.state.LocalAppState
import se.bofr.dspi.usb.BootloaderLocator

/** Test tag of the connection strip, the top of the app. */
internal const val CONNECTION_STRIP_TAG = "connection-strip"

/** Test tag of the strip's "Devices" button (WP AN7). */
internal const val DEVICE_LIST_TAG = "device-list"

/** Test tag of the compact strip's overflow button (WP AN42): the strip's actions and the device list. */
internal const val STRIP_MENU_TAG = "strip-menu"

/**
 * macOS window/menu structure -> one NavHost (PLAN §3.3 `ui/nav/AppScaffold.kt`).
 * Every route of [DspiRoutes] maps to exactly one feature entry composable.
 * The NavHost is keyed on [bridge]: entering or leaving demo mode starts the
 * navigation over, so no screen holds state from the other bridge.
 *
 * [bootloaderLocator]: the app's one locator, given to FirmwareUpdateView.
 *
 * Compat banner: the contract exposes it on the bridge (`compat`,
 * `compatMessage`, refreshed with `state`), HOME draws it; the app adds
 * nothing but this strip for the USB/demo states the bridge cannot know.
 *
 * Device list (WP AN7): the strip's "Devices" menu lists [devices] (USB, DSPi
 * Link, demo; [deviceList]). [onChooseDevice] gets the user's pick, except
 * "DSPi Link…", which is [onConnectNetwork]. [switchAlerts] is where a switch's
 * Unsaved Changes alert shows ([AppModeController.switchAlerts]).
 *
 * No title bar (AN27): the strip is the top of the app, and it is what says
 * demo mode. With no topBar the Scaffold's content padding comes from its
 * contentWindowInsets alone, so those keep the strip off the status bar. Screens
 * handle no insets themselves, so the same padding keeps them off a landscape cutout.
 *
 * Hardware keys (WP AN35): the app's column holds focus when nothing inside
 * it does, and takes the keys its focused view leaves ([ConsoleKeyMap]).
 */
@Composable
fun AppScaffold(
    bridge: DspiBridge,
    strip: ConnectionStrip,
    onExitDemo: () -> Unit,
    onRequestPermission: () -> Unit,
    bootloaderLocator: BootloaderLocator? = null,
    onRemoteAction: (RemoteAction) -> Unit = {},
    onConnectNetwork: () -> Unit = {},
    // -- WP AN19 -- the core's onboarding data; null: no first-run pages (tests)
    onboarding: OnboardingSource? = null,
    // -- WP AN7 --
    devices: List<DeviceRow> = emptyList(),
    onChooseDevice: (DeviceChoice) -> Unit = {},
    switchAlerts: AlertHost? = null,
    overlay: @Composable () -> Unit = {}
) {
    // -- WP AN35 --
    val keys = remember { ShortcutKeys() }
    val view = LocalView.current
    LaunchedEffect(keys, view) { keys.editing = { view.onCheckIsTextEditor() } } // -- WP P1AN1 --
    val focus = remember { FocusRequester() }
    var focusLost by remember { mutableIntStateOf(0) }
    // -- WP AN41 -- a write the core refused, in its words, over the screen
    val notices = remember { SnackbarHostState() }
    RefusalNotices(bridge, notices)
    // -- WP AN42 -- Undo and Redo: the strip's buttons, the keys, their snackbar;
    // the drag mark every slider and graph drag holds, ended whenever the app stops
    val historyScope = rememberCoroutineScope()
    val history = remember(bridge) { UndoRedoModel(bridge, historyScope) }
    UndoRedoNotices(history, notices)
    val dragMarker = remember(bridge) { DragMarker(bridge::setDragging) }
    WhileVisible(key = dragMarker, onShow = {}, onHide = { dragMarker.endAll() })
    // The IME stays out, as with the Scaffold's default insets
    Scaffold(
        snackbarHost = { RefusalNoticeHost(notices) },
        contentWindowInsets = WindowInsets.systemBars.union(WindowInsets.displayCutout)
    ) { padding ->
        // -- WP P1AN1 -- the safe apply's bar floats over the screens (no layout growth)
        Box(Modifier.fillMaxSize().padding(padding)) {
            Column(
                Modifier
                    .fillMaxSize()
                    .onKeyEvent(keys::onKey) // -- WP AN35 --
                    .focusRequester(focus)
                    .onFocusChanged { if (!it.hasFocus) focusLost++ }
                    .focusable()
            ) {
                // In here: the Scaffold composes its content after the scaffold itself, the requester's node with it
                LaunchedEffect(focusLost) { focus.requestFocus() }
                ConnectionStripBar(
                    strip,
                    onExitDemo,
                    onRequestPermission,
                    onRemoteAction,
                    history,
                    devices,
                    // -- WP P1AN1 -- the app bar's Snapshots entry
                    onSnapshots = { keys.host?.let { h -> if (h.currentRoute != DspiRoutes.SNAPSHOTS) h.navigate(DspiRoutes.SNAPSHOTS) } }
                ) { c ->
                    if (c == DeviceChoice.Network) onConnectNetwork() else onChooseDevice(c)
                }
                key(bridge) {
                    // -- WP AN35 -- the host's USB format, for every route; the DSPi is on this phone's USB only outside demo and Link
                    HostChannelsFeed(
                        bridge,
                        enabled = strip !is ConnectionStrip.Demo && strip !is ConnectionStrip.Remote && strip !is ConnectionStrip.Document
                    )
                    CompositionLocalProvider(
                        // -- WP P1AN1 -- a document opened offline has no device either
                        LocalDemoMode provides (strip is ConnectionStrip.Demo || strip is ConnectionStrip.Document),
                        LocalOnboarding provides onboarding, // -- WP AN19 --
                        LocalDragMarker provides dragMarker // -- WP AN42 --
                    ) {
                        DspiNavHost(bridge, bootloaderLocator, remote = strip is ConnectionStrip.Remote, keys, history)
                    }
                }
            }
            key(bridge) { SafeApplyBar(bridge, Modifier.align(Alignment.BottomCenter)) } // -- WP P1AN1 --
        }
    }
    overlay()
    switchAlerts?.let { ConsoleAlertHost(it) } // -- WP AN7 --
}

@Composable
private fun DspiNavHost(
    bridge: DspiBridge,
    bootloaderLocator: BootloaderLocator?,
    remote: Boolean,
    keys: ShortcutKeys,
    // -- WP AN42 --
    history: UndoRedoModel
) {
    // G1: every device-writing control is disabled while the bridge refuses writes
    val writes by bridge.writesAllowed.collectAsStateWithLifecycle()
    CompositionLocalProvider(LocalWritesAllowed provides writes) { DspiRoutesHost(bridge, bootloaderLocator, remote, keys, history) }
}

@Composable
private fun DspiRoutesHost(
    bridge: DspiBridge,
    bootloaderLocator: BootloaderLocator?,
    remote: Boolean,
    keys: ShortcutKeys,
    // -- WP AN42 --
    history: UndoRedoModel
) {
    val nav = rememberNavController()
    // -- WP AN35 -- the shortcuts drive this NavHost while it is on screen
    val writes = rememberUpdatedState(LocalWritesAllowed.current)
    val appState = LocalAppState.current
    val scope = rememberCoroutineScope()
    val keyAlerts = remember(bridge) { AlertHost() } // -- WP P1AN1 -- the preset and Commit keys' alerts
    ConsoleAlertHost(keyAlerts)
    DisposableEffect(nav, bridge, appState, history, keyAlerts) {
        keys.host = NavShortcutHost(nav, bridge, appState, scope, history, keyAlerts) { writes.value }
        onDispose { keys.host = null }
    }
    NavHost(navController = nav, startDestination = DspiRoutes.HOME) {
        // Each screen's back/navigate act only while its own entry is current (EntryNavigator)
        screen(nav, DspiRoutes.HOME) { _, go, back -> HomeScreen(bridge, go, back) }
        screen(nav, DspiRoutes.CHANNEL, listOf(navArgument("ch") { type = NavType.IntType })) { entry, go, back ->
            ChannelScreen(bridge, entry.arguments?.getInt("ch") ?: 0, go, back)
        }
        screen(nav, DspiRoutes.GRAPH) { _, go, back -> GraphScreen(bridge, go, back) }
        screen(nav, DspiRoutes.MATRIX) { _, go, back -> MatrixMixerView(bridge, go, back) }
        screen(nav, DspiRoutes.TOOLS) { _, go, back -> ToolsScreen(bridge, go, back) }
        // -- WP AN31 -- a quick-strip long-press opens that tool's page directly
        screen(nav, DspiRoutes.TOOL, listOf(navArgument("tool") { type = NavType.StringType })) { entry, go, back ->
            ToolsScreen(bridge, go, back, open = entry.arguments?.getString("tool"))
        }
        screen(nav, DspiRoutes.LIMITER, listOf(navArgument("out") { type = NavType.IntType })) { entry, go, back ->
            OutputLimiterView(bridge, entry.arguments?.getInt("out") ?: 0, go, back)
        }
        screen(nav, DspiRoutes.PRESETS) { _, go, back -> PresetsScreen(bridge, go, back) }
        // -- WP P1AN1 --
        screen(nav, DspiRoutes.SNAPSHOTS) { _, go, back -> SnapshotsScreen(bridge, go, back) }
        screen(nav, DspiRoutes.SETTINGS) { _, go, back -> SettingsScreen(bridge, go, back) }
        // -- WP LKUI1 -- the hub's audio link installation, from Home's status chip
        screen(nav, DspiRoutes.SYSTEM) { _, go, back -> SystemScreen(bridge, go, back) }
        screen(nav, DspiRoutes.ANALYSIS) { _, go, back -> AnalysisScreen(bridge, go, back) }
        // -- WP AN32 -- each Tools-menu entry opens its own analysis page
        screen(nav, DspiRoutes.ANALYSIS_PAGE, listOf(navArgument("page") { type = NavType.StringType })) { entry, go, back ->
            AnalysisScreen(bridge, go, back, open = entry.arguments?.getString("page"))
        }
        screen(nav, DspiRoutes.FIRMWARE) { _, go, back -> FirmwareRoute(bridge, go, back, bootloaderLocator, remote) }
        // -- WP AN19 --
        screen(nav, DspiRoutes.GETTING_STARTED) { _, go, back -> GettingStartedRoute(bridge, go, back) }
        screen(nav, DspiRoutes.WHATS_NEW) { _, _, back -> WhatsNewRoute(back) }
    }
    OnboardingAtLaunch(nav) // -- WP AN19 --
}

@Composable
internal fun FirmwareRoute(
    bridge: DspiBridge,
    go: (String) -> Unit,
    back: () -> Unit,
    bootloaderLocator: BootloaderLocator?,
    remote: Boolean
) {
    when {
        // AN28: the updater looks for boards in BOOTSEL on this phone's USB; demo mode touches no device
        LocalDemoMode.current -> FirmwareNotice(
            back,
            "This is the demo device, so there is no firmware to update. To update a DSPi, exit demo and connect it."
        )
        // DSPi Link: the board's bootloader is on the hub's USB, not this phone's (no fw.install client)
        remote -> FirmwareNotice(
            back,
            "This DSPi is connected through DSPi Link. Update its firmware in the DSPi Console it is plugged into, " +
                "or connect it over USB."
        )
        // The firmware update path stays usable read-only (G1: enterBootloader is the one write the bridge allows)
        else -> WritesExempt { FirmwareUpdateView(bridge, go, back, bootloaderLocator = bootloaderLocator) }
    }
}

private fun NavGraphBuilder.screen(
    nav: NavHostController,
    route: String,
    arguments: List<NamedNavArgument> = emptyList(),
    content: @Composable (entry: NavBackStackEntry, go: (String) -> Unit, back: () -> Unit) -> Unit
) {
    composable(route, arguments) { entry ->
        val navigator = remember(nav, entry) { nav.navigatorFor(entry) }
        content(entry, navigator::go, navigator::back)
    }
}

// -- WP P1AN1 --
/** The app bar's entry to the Snapshots screen. */
internal const val SNAPSHOTS_ENTRY = "Snapshots"

/** Test tag of the wide strip's Snapshots button. */
internal const val SNAPSHOTS_ENTRY_TAG = "strip-snapshots"

/** Below this width the strip keeps to one line: its actions and the device list go into an overflow menu (WP AN42). */
internal val STRIP_COMPACT_BELOW = 600.dp

@Composable
private fun ConnectionStripBar(
    strip: ConnectionStrip,
    onExitDemo: () -> Unit,
    onRequestPermission: () -> Unit,
    onRemoteAction: (RemoteAction) -> Unit,
    // -- WP AN42 --
    history: UndoRedoModel?,
    // -- WP AN7 --
    devices: List<DeviceRow>,
    // -- WP P1AN1 --
    onSnapshots: (() -> Unit)? = null,
    onChooseDevice: (DeviceChoice) -> Unit
) {
    // -- WP AN5 -- the owner's lost-connection banner; this bar is above every route, so it is on every screen
    if (strip is ConnectionStrip.Remote && strip.lost) {
        LinkBanner(strip.text, strip.detail, strip.actions.map { a -> LinkBannerAction(a.label) { onRemoteAction(a) } })
        return
    }
    // Demo and REMOTE stand out, so neither is taken for the USB unit
    val demo = strip is ConnectionStrip.Demo || strip is ConnectionStrip.Remote || strip is ConnectionStrip.Document // -- WP P1AN1 --
    val colors = MaterialTheme.colorScheme
    val ink = if (demo) colors.onTertiaryContainer else colors.onSurfaceVariant
    val actions: List<Pair<String, () -> Unit>> =
        when (strip) {
            ConnectionStrip.Demo -> listOf("Exit demo" to onExitDemo)
            is ConnectionStrip.Document -> listOf("Close file" to onExitDemo) // -- WP P1AN1 --
            is ConnectionStrip.Remote -> strip.actions.map { a -> a.label to { onRemoteAction(a) } }
            is ConnectionStrip.NeedsPermission -> listOf("Allow USB access" to onRequestPermission)
            ConnectionStrip.NoDevice, ConnectionStrip.Usb, is ConnectionStrip.Info -> emptyList()
        }
    BoxWithConstraints(
        Modifier
            .testTag(CONNECTION_STRIP_TAG)
            .fillMaxWidth()
            .background(if (demo) colors.tertiaryContainer else colors.surfaceVariant)
    ) {
        // -- WP AN42 -- a phone: one line (the short text, the whole of it on a long-press and to
        // TalkBack), Undo and Redo, and the strip's actions and devices behind the overflow
        val compact = maxWidth < STRIP_COMPACT_BELOW
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, end = if (compact) 4.dp else 16.dp, top = 4.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            if (compact) {
                HelpTip(if (strip.short != strip.text) strip.text else "", Modifier.weight(1f)) {
                    Text(
                        strip.short,
                        Modifier.semantics { if (strip.short != strip.text) contentDescription = strip.text },
                        style = MaterialTheme.typography.bodyMedium,
                        color = ink,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
                history?.let { UndoRedoButtons(it, ink) }
                // -- WP P1AN1 -- Snapshots first in the overflow: the bar keeps its one line
                StripMenu(listOfNotNull(onSnapshots?.let { SNAPSHOTS_ENTRY to it }) + actions, devices, onChooseDevice, ink)
            } else {
                Text(strip.text, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = ink)
                actions.forEach { (label, act) -> TextButton(onClick = act) { Text(label) } }
                // -- WP P1AN1 --
                onSnapshots?.let { TextButton(onClick = it, Modifier.testTag(SNAPSHOTS_ENTRY_TAG)) { Text(SNAPSHOTS_ENTRY) } }
                history?.let { UndoRedoButtons(it, ink) } // -- WP AN42 --
                DeviceListButton(devices, onChooseDevice) // -- WP AN7 --
            }
        }
    }
}

// -- WP AN7 --
/** "Devices": the one list of every way to a DSPi, the current one ticked. Nothing while [rows] is empty. */
@Composable
private fun DeviceListButton(rows: List<DeviceRow>, onChoose: (DeviceChoice) -> Unit) {
    if (rows.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }, Modifier.testTag(DEVICE_LIST_TAG)) { Text("Devices") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DeviceRows(rows, onChoose) { open = false }
        }
    }
}

@Composable
private fun DeviceRows(rows: List<DeviceRow>, onChoose: (DeviceChoice) -> Unit, close: () -> Unit) {
    rows.forEach { r ->
        DropdownMenuItem(
            text = { Text(if (r.current) "✓ ${r.label}" else r.label) },
            enabled = r.enabled,
            onClick = {
                close()
                if (!r.current) onChoose(r.choice)
            }
        )
    }
}

// -- WP AN42 --
/** The compact strip's overflow (⋮): its actions first, then the device list. Nothing when both are empty. */
@Composable
private fun StripMenu(actions: List<Pair<String, () -> Unit>>, rows: List<DeviceRow>, onChoose: (DeviceChoice) -> Unit, tint: Color) {
    if (actions.isEmpty() && rows.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }, Modifier.testTag(STRIP_MENU_TAG)) {
            Icon(DspiIcons.more, contentDescription = "More", tint = tint)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            actions.forEach { (label, act) ->
                DropdownMenuItem(text = { Text(label) }, onClick = {
                    open = false
                    act()
                })
            }
            if (actions.isNotEmpty() && rows.isNotEmpty()) HorizontalDivider()
            if (rows.isNotEmpty()) {
                Text(
                    "Devices",
                    Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            DeviceRows(rows, onChoose) { open = false }
        }
    }
}

@Composable
private fun FirmwareNotice(back: () -> Unit, text: String) {
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = back) { Text("‹ Back") }
        Text("Firmware Update", style = MaterialTheme.typography.headlineSmall)
        Text(text, style = MaterialTheme.typography.bodyLarge)
    }
}

// -- WP AN35 --
/** Hands the column's keys to the current NavHost's [ShortcutHost]. */
internal class ShortcutKeys {
    var host: ShortcutHost? = null

    // -- WP P1AN1 -- a text field has the keyboard (the Compose view's text input session)
    var editing: () -> Boolean = { false }

    fun onKey(e: KeyEvent): Boolean {
        if (e.type != KeyEventType.KeyDown || e.nativeKeyEvent.repeatCount > 0) return false
        val shortcut = ConsoleKeyMap.shortcutOf(e.key, e.isCtrlPressed || e.isMetaPressed, e.isShiftPressed, e.isAltPressed)
            ?: return false
        if (shortcut.notWhileEditing && editing()) return false // -- WP P1AN1 --
        return host?.let { ConsoleShortcuts.perform(shortcut, it) } ?: false
    }
}

/** [ShortcutHost] over the app's NavHost, the bridge and the main window's selection. */
private class NavShortcutHost(
    private val nav: NavHostController,
    private val bridge: DspiBridge,
    private val appState: AppState,
    private val scope: CoroutineScope,
    // -- WP AN42 --
    private val history: UndoRedoModel,
    // -- WP P1AN1 --
    private val alerts: AlertHost,
    private val writes: () -> Boolean
) : ShortcutHost {
    override val currentRoute: String?
        get() = nav.currentBackStackEntry?.let { e ->
            e.destination.route?.let { p -> ConsoleShortcuts.resolve(p) { name -> e.arguments?.get(name)?.toString() } }
        }
    override val selectedChannel: Int? get() = appState.activeEqChannel.value
    override val writesAllowed: Boolean get() = writes()

    override fun canPaste() = bridge.canPaste()

    override fun navigate(route: String) = nav.navigate(route)

    override fun copyChannel(ch: Int) = bridge.copyChannel(ch)

    override fun pasteChannel(ch: Int) = bridge.pasteChannel(ch)

    override val generatorRunning: Boolean get() = bridge.siggen.status.value.isRunning

    /** The page's own `startBlocker`; no draft yet (no caps read) does not start. */
    override val generatorCanStart: Boolean
        get() {
            val draft = bridge.siggen.draft.value ?: return false
            val page = TestSignalsUiState(
                connected = bridge.status.value.connected,
                caps = bridge.siggen.caps.value,
                draft = draft,
                status = bridge.siggen.status.value,
                outputNames = emptyList(),
                outputEnabled = emptyList()
            )
            return page.startBlocker == null
        }

    override fun startGenerator() {
        scope.launch { bridge.siggen.start() }
    }

    override fun stopGenerator() = bridge.siggen.stop()

    // -- WP AN42 --
    override fun undo() = history.undo()

    override fun redo() = history.redo()

    // -- WP P1AN1 -- as the Presets page's menu actions, their alerts over every screen
    override fun loadPreset(slot: Int): Boolean {
        if (!bridge.connected || slot !in 0 until PRESET_SLOT_COUNT) return false
        PresetMenuActions(bridge, scope, alerts).selectPreset(slot)
        return true
    }

    override fun commit(): Boolean {
        if (!bridge.connected) return false
        ToolsMenuActions(bridge, scope, alerts).commitParameters()
        return true
    }

    override fun revert(): Boolean {
        if (!bridge.connected) return false
        ToolsMenuActions(bridge, scope, alerts).revertToSaved()
        return true
    }

    override fun toggleMute(): Boolean {
        if (!bridge.connected) return false
        bridge.setUserMute(!bridge.userMute)
        return true
    }

    override fun toggleBypass(): Boolean {
        if (!bridge.connected) return false
        bridge.setBypass(!bridge.bypass)
        return true
    }

    override fun abToggle(): Boolean {
        val snaps = bridge.snapshots
        if (!snaps.compare.value.active || snaps.guardWaiting) return false
        scope.launch { snaps.abToggle() }
        return true
    }
}
