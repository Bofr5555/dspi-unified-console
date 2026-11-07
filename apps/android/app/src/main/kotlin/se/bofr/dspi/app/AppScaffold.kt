package se.bofr.dspi.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
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
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.analysis.AnalysisScreen
import se.bofr.dspi.feature.firmware.FirmwareUpdateView
import se.bofr.dspi.feature.graph.GraphScreen
import se.bofr.dspi.feature.home.ChannelScreen
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.feature.matrix.MatrixMixerView
import se.bofr.dspi.feature.onboarding.OnboardingSource
import se.bofr.dspi.feature.presets.PresetsScreen
import se.bofr.dspi.feature.settings.SettingsScreen
import se.bofr.dspi.feature.tools.OutputLimiterView
import se.bofr.dspi.feature.tools.ToolsScreen
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.LocalDemoMode
import se.bofr.dspi.ui.LocalWritesAllowed
import se.bofr.dspi.ui.WritesExempt
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.ConsoleAlertHost
import se.bofr.dspi.ui.link.LinkBanner
import se.bofr.dspi.ui.link.LinkBannerAction
import se.bofr.dspi.usb.BootloaderLocator

/** Test tag of the connection strip, the top of the app. */
internal const val CONNECTION_STRIP_TAG = "connection-strip"

/** Test tag of the strip's "Devices" button (WP AN7). */
internal const val DEVICE_LIST_TAG = "device-list"

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
    // The IME stays out, as with the Scaffold's default insets
    Scaffold(contentWindowInsets = WindowInsets.systemBars.union(WindowInsets.displayCutout)) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            ConnectionStripBar(strip, onExitDemo, onRequestPermission, onRemoteAction) {
                DeviceListButton(devices) { c -> if (c == DeviceChoice.Network) onConnectNetwork() else onChooseDevice(c) }
            }
            key(bridge) {
                CompositionLocalProvider(
                    LocalDemoMode provides (strip is ConnectionStrip.Demo),
                    LocalOnboarding provides onboarding // -- WP AN19 --
                ) {
                    DspiNavHost(bridge, bootloaderLocator, remote = strip is ConnectionStrip.Remote)
                }
            }
        }
    }
    overlay()
    switchAlerts?.let { ConsoleAlertHost(it) } // -- WP AN7 --
}

@Composable
private fun DspiNavHost(bridge: DspiBridge, bootloaderLocator: BootloaderLocator?, remote: Boolean) {
    // G1: every device-writing control is disabled while the bridge refuses writes
    val writes by bridge.writesAllowed.collectAsStateWithLifecycle()
    CompositionLocalProvider(LocalWritesAllowed provides writes) { DspiRoutesHost(bridge, bootloaderLocator, remote) }
}

@Composable
private fun DspiRoutesHost(bridge: DspiBridge, bootloaderLocator: BootloaderLocator?, remote: Boolean) {
    val nav = rememberNavController()
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
        screen(nav, DspiRoutes.SETTINGS) { _, go, back -> SettingsScreen(bridge, go, back) }
        screen(nav, DspiRoutes.ANALYSIS) { _, go, back -> AnalysisScreen(bridge, go, back) }
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
            "This DSPi is connected through DSPi Link. Update its firmware on the computer it is plugged into " +
                "(DSPi Console), or connect it to this phone over USB."
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

@Composable
private fun ConnectionStripBar(
    strip: ConnectionStrip,
    onExitDemo: () -> Unit,
    onRequestPermission: () -> Unit,
    onRemoteAction: (RemoteAction) -> Unit,
    devices: @Composable () -> Unit
) {
    // -- WP AN5 -- the owner's lost-connection banner; this bar is above every route, so it is on every screen
    if (strip is ConnectionStrip.Remote && strip.lost) {
        LinkBanner(strip.text, strip.detail, strip.actions.map { a -> LinkBannerAction(a.label) { onRemoteAction(a) } })
        return
    }
    // Demo and REMOTE stand out, so neither is taken for the USB unit
    val demo = strip is ConnectionStrip.Demo || strip is ConnectionStrip.Remote
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier
            .testTag(CONNECTION_STRIP_TAG)
            .fillMaxWidth()
            .background(if (demo) colors.tertiaryContainer else colors.surfaceVariant)
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(
            strip.text,
            Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
            color = if (demo) colors.onTertiaryContainer else colors.onSurfaceVariant
        )
        when (strip) {
            ConnectionStrip.Demo -> TextButton(onClick = onExitDemo) { Text("Exit demo") }
            is ConnectionStrip.Remote -> Row {
                strip.actions.forEach { a -> TextButton(onClick = { onRemoteAction(a) }) { Text(a.label) } }
            }
            is ConnectionStrip.NeedsPermission -> TextButton(onClick = onRequestPermission) { Text("Allow USB access") }
            ConnectionStrip.NoDevice, ConnectionStrip.Usb, is ConnectionStrip.Info -> Unit
        }
        devices() // -- WP AN7 --
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
            rows.forEach { r ->
                DropdownMenuItem(
                    text = { Text(if (r.current) "✓ ${r.label}" else r.label) },
                    enabled = r.enabled,
                    onClick = {
                        open = false
                        if (!r.current) onChoose(r.choice)
                    }
                )
            }
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
