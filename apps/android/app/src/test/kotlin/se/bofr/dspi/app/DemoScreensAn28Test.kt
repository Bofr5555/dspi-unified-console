package se.bofr.dspi.app

import android.app.Application
import android.content.Context
import android.hardware.usb.UsbDevice
import android.os.Looper
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.analysis.AnalysisScreen
import se.bofr.dspi.feature.graph.GraphScreen
import se.bofr.dspi.feature.home.ChannelScreen
import se.bofr.dspi.feature.home.HomeScreen
import se.bofr.dspi.feature.matrix.MatrixMixerView
import se.bofr.dspi.feature.presets.PresetsScreen
import se.bofr.dspi.feature.settings.SettingsCategory
import se.bofr.dspi.feature.settings.SettingsView
import se.bofr.dspi.feature.tools.OutputLimiterView
import se.bofr.dspi.feature.tools.ToolsScreen
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.LocalDemoMode
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.usb.BootloaderDevice
import se.bofr.dspi.usb.BootloaderLocator
import se.bofr.dspi.usb.BootloaderPermissionResult

/**
 * WP AN28: every main screen of demo mode on the real core (host build of
 * libdspi_ffi over JNA), through the real bridge. No device.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class DemoScreensAn28Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ffiCore = FfiDspiCore()
    private lateinit var session: DemoSession
    private lateinit var bridge: DspiBridge
    private val failures = mutableListOf<String>()

    @After
    fun stop() {
        if (::session.isInitialized) rule.runOnUiThread { session.close() } else ffiCore.close()
        scope.cancel()
    }

    /** The bridge posts to the main looper, which nothing else pumps here; the FFI thread runs in real time. */
    private fun pump(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (!until()) {
            check(System.currentTimeMillis() < end) { "timed out: $what" }
            shadowOf(Looper.getMainLooper()).idle()
            Thread.sleep(10)
        }
    }

    private fun settle() = repeat(30) {
        shadowOf(Looper.getMainLooper()).idle()
        rule.mainClock.advanceTimeBy(100)
        Thread.sleep(5)
    }

    private fun start() {
        session = startDemoSession(scope, "an28", core = DemoCore(ffiCore))
        bridge = session.bridge
        bridge.events.onEach { if (it is BridgeEvent.CommandFailed) failures += "${it.command}: ${it.error}" }.launchIn(scope)
        // Compose does not go idle on Home with the clock on auto (as NoTitleBarAn27Test)
        rule.mainClock.autoAdvance = false
        pump("connected") { bridge.status.value.connected }
    }

    /** One composition, so screens swap as they do under the NavHost. */
    private fun host(demo: Boolean = true): (@Composable () -> Unit) -> Unit {
        var current by mutableStateOf<(@Composable () -> Unit)?>(null)
        rule.setContent {
            CompositionLocalProvider(LocalDemoMode provides demo) {
                MaterialTheme(colorScheme = darkColorScheme()) { Surface(Modifier.fillMaxSize()) { current?.invoke() } }
            }
        }
        return { screen ->
            rule.runOnUiThread { current = null }
            settle()
            rule.runOnUiThread { current = screen }
            settle()
        }
    }

    private fun assertShows(screen: String, vararg texts: String) {
        for (t in texts) {
            val found = rule.onAllNodesWithText(t, substring = true, useUnmergedTree = true).fetchSemanticsNodes()
            assertTrue("$screen shows \"$t\"", found.isNotEmpty())
        }
        assertEquals("$screen: no command failed", emptyList<String>(), failures)
    }

    private fun assertAbsent(text: String) = assertTrue(
        "\"$text\" is not on screen",
        rule.onAllNodesWithText(text, substring = true, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()
    )

    @Test
    fun an28EveryMainScreenRendersOnTheRealDemo() {
        start()
        assertTrue("the demo device takes writes", bridge.writesAllowed.value)
        val show = host()
        show { HomeScreen(bridge, {}, {}) }
        assertShows("home", "Filter Response", "USB 1", "Demo", "User Volume")
        // -- WP INTD -- (REVD W12) output 1's short label is "L": a full-range output says no "· Full"
        assertTrue("home shows \"L\"", rule.onAllNodesWithText("L", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        assertAbsent("· Full")
        assertAbsent("· None")
        show { ChannelScreen(bridge, 0, {}, {}) }
        assertShows("input channel", "Preamp", "Clear PEQ", "Bypass All")
        show { ChannelScreen(bridge, 2, {}, {}) }
        assertShows("output channel", "GAIN", "DELAY", "Clear All")
        show { GraphScreen(bridge, {}, {}) }
        assertShows("graph", "Response Graph", "Following: OUT1")
        show { MatrixMixerView(bridge, {}, {}) }
        assertShows("matrix", "Matrix Mixer", "ROUTING", "OUTPUT")
        show { OutputLimiterView(bridge, 0, {}, {}) }
        assertShows("limiter", "Output Limiter", "Threshold", "Release")
        show { PresetsScreen(bridge, {}, {}) }
        assertShows("presets", "Presets & Files", "1: Demo", "Export Device Configuration")
    }

    // A tall window, so every tool row is on screen without scrolling: AN31 put two rows above the
    // tools, and performScrollTo animates and hangs on the paused clock (as ToolsEntryAn31Test).
    @Test
    @Config(qualifiers = "w400dp-h1600dp-port-mdpi")
    fun an28EveryToolRendersOnTheRealDemo() {
        start()
        val show = host()
        val tools = mapOf(
            "Loudness Compensation" to "Reference",
            "Headphone Crossfeed" to "Preset",
            "Psychoacoustic Bass" to "Phantom",
            "Subharmonic Synthesizer" to "half the frequency",
            "Tube Modeller" to "Valve",
            "Stereo Upmixer" to "Correlation",
            "Volume Leveller" to "Amount"
        )
        for ((tool, inside) in tools) {
            show { ToolsScreen(bridge, {}, {}) }
            assertShows("tools list", tool)
            rule.onNodeWithText(tool).assertIsDisplayed().performClick()
            // The click's state write reaches composition through an apply notification; send it
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            settle()
            assertShows(tool, "‹ Tools", inside)
        }
    }

    @Test
    fun an28EverySettingsPageRendersOnTheRealDemo() {
        start()
        // A snapshot the pages read; Control Surfaces, Groups, Macros and Aux are CG16's, which the Kotlin demo never had
        pump("surfaces") { bridge.cs.loaded.value }
        val store = InMemoryAppSettingsStore()
        val show = host()
        val pages = mapOf(
            SettingsCategory.GENERAL to "Licences",
            SettingsCategory.ADVANCED to "Channel Names",
            SettingsCategory.GRAPHING to "Graph Appearance",
            SettingsCategory.SPECTRUM to "This device reports 120 dB of usable range",
            SettingsCategory.OVERVIEW to "Core Voltage",
            SettingsCategory.SPDIF_INPUT to "S/PDIF Input",
            SettingsCategory.OUTPUT_ASSIGNMENT to "Output 1",
            SettingsCategory.I2S_CONFIG to "I2S Clock",
            SettingsCategory.GLOBAL_PARAMS to "Serial $DEMO_DEVICE_SERIAL",
            SettingsCategory.CONTROL_SURFACES to "Volume Knob",
            SettingsCategory.CHANNEL_GROUPS to "Front Pair",
            SettingsCategory.MACROS to "Night",
            SettingsCategory.AUX_OUTPUTS to "Amp Trigger"
        )
        for ((page, inside) in pages) {
            show { SettingsView(bridge, store, {}, {}, initialPage = page) }
            assertShows(page.title, inside)
        }
    }

    /** Settle until [until], for what the FFI thread delivers in real time (the analyser, the generator's status). */
    private fun settleUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 15_000
        while (!until()) {
            check(System.currentTimeMillis() < end) { "timed out: $what" }
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(100)
            Thread.sleep(10)
        }
    }

    private fun onScreen(text: String) =
        rule.onAllNodesWithText(text, substring = true, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    /** WP CG22: the core's demo device has the generator and the analyser again, as the Kotlin demo had. */
    @Test
    @Config(qualifiers = "w400dp-h1600dp-port-mdpi")
    fun cg22AnalysisShowsTheGeneratorAndTheAnalyserOnTheDemo() {
        start()
        val show = host()
        show { AnalysisScreen(bridge, {}, {}) }
        settleUntil("generator caps") { bridge.siggenSupported }
        assertShows("signal generator", "Signal Generator", "OUTPUTS", "LEVEL", "Start")
        assertAbsent("Signal generator not available")
        assertEquals("the generator never starts by itself", 0, ffiCore.siggenGetStatus().state)
        rule.onAllNodesWithText("All")[0].performClick()
        settle()
        rule.onNodeWithText("Start").performClick()
        settleUntil("playing") { ffiCore.siggenGetStatus().state != 0 }
        settleUntil("the Stop button") { onScreen("Stop") }
        rule.onNodeWithText("Stop").performClick()
        settleUntil("stopped") { ffiCore.siggenGetStatus().state == 0 }
        rule.onAllNodesWithText("Spectrum Analyser")[0].performClick()
        settle()
        settleUntil("analyser frames") { onScreen("frames/s") }
        assertShows("analyser", "Running", "frames/s")
        assertAbsent("Spectrum analyser unavailable")
        assertTrue("frames arrive", bridge.rta.snapshot.value?.running == true)
        rule.onAllNodesWithText("System Statistics")[0].performClick()
        settle()
        // The core's ring-health numbers and fill_band, which the Kotlin demo only imitated
        assertShows("statistics", "RP2350", "307.2 MHz", "OUTPUT RING HEALTH", "192 frames (4 ms)", "38–38%")
        rule.onAllNodesWithText("Interrupt Monitor")[0].performClick()
        settle()
        assertEquals(emptyList<String>(), failures)
    }

    /** The real core's rules, not "accepts anything": a write lands, and a control that does not exist is refused. */
    @Test
    fun an28TheDemoBehavesLikeTheCore() {
        start()
        host()
        bridge.setPreamp(-3f)
        pump("preamp") { bridge.state.value.preampDb == -3f }
        pump("surfaces") { bridge.cs.loaded.value }
        val refused = JSONObject(ffiCore.csApply("""{"op":"name","index":99,"name":"x"}"""))
        assertFalse("an unknown control is refused", refused.getBoolean("ok"))
        assertEquals(emptyList<String>(), failures)
    }

    @Test
    fun an28TheStripSaysDemoAndStaysAcrossScreens() {
        start()
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Demo, onExitDemo = {}, onRequestPermission = {}) }
        settle()
        val inStrip = hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))
        rule.onNode(hasText(ConnectionStrip.Demo.short) and inStrip).assertIsDisplayed()
        // -- WP AN42 -- on a phone the way out is in the strip's overflow
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        settle()
        rule.onNodeWithText("Exit demo").assertIsDisplayed().performClick()
        settle()
        rule.onNodeWithText("Filter Response").assertIsDisplayed()
        // -- WP UIM1 -- the phone's Home has the tool icons below the graph and the channel list, off
        // screen here: the click action, as TalkBack's (performScrollTo hangs on the paused clock)
        rule.onNode(hasContentDescription("Settings")).performSemanticsAction(SemanticsActions.OnClick)
        settle()
        rule.onNodeWithText("Application").assertIsDisplayed()
        rule.onNode(hasText(ConnectionStrip.Demo.short) and inStrip).assertIsDisplayed()
        assertEquals(emptyList<String>(), failures)
    }

    @Test
    fun an28TheFirmwarePageNeverLooksForABoardInDemo() {
        start()
        val locator = object : BootloaderLocator {
            var used = false
            override val devices = MutableStateFlow(emptyList<BootloaderDevice>())
            override val permissionResults = MutableSharedFlow<BootloaderPermissionResult>()

            override fun start(context: Context) {
                used = true
            }

            override fun stop(context: Context) = Unit

            override fun hasPermission(device: BootloaderDevice) = false

            override fun requestPermission(device: BootloaderDevice) = Unit

            override fun usbDevice(device: BootloaderDevice): UsbDevice? = null
        }
        val show = host()
        show { FirmwareRoute(bridge, {}, {}, locator, remote = false) }
        assertShows("firmware", "This is the demo device")
        assertAbsent("Update Firmware")
        assertFalse("the locator was never started", locator.used)
    }

    @Test
    fun an28LeavingClosesTheRealCoreOffTheMainThread() {
        start()
        host()
        rule.runOnUiThread { session.close() }
        // -- WP FLAKE2 -- by prefix: while it runs a coroutine, kotlinx's debug mode (on with -ea, as
        // in Gradle's tests) names the thread "dspi-demo-ffi @coroutine#N", and an exact match
        // took a thread still working off its queue, before the close, for one that had ended
        pump("the FFI thread ends") { Thread.getAllStackTraces().keys.none { it.name.startsWith("dspi-demo-ffi") && it.isAlive } }
        assertThrows("the core is freed", IllegalStateException::class.java) { ffiCore.isConnected() }
    }
}
