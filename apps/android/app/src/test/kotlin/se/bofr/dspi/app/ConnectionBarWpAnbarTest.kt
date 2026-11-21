package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.ui.BarColors
import se.bofr.dspi.ui.DspiBarColors
import se.bofr.dspi.ui.DspiColors
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.usb.UsbIds
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP ANBAR (owner 2026-11-21): the connection bar in each of its states, a phone and a wide
 * window, light and dark: the app's own tints (no Material default: its tertiary is pink), at
 * most 40dp on one line, every button on it still touched over 48dp. With
 * `-Pdspi.shotsDir=<dir>` each is shot (the top of the window down to just below the bar) and
 * its height written to `bar-heights-<size>-<theme>.txt` there.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ConnectionBarWpAnbarTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)
    private val store = InMemoryAppSettingsStore(AppSettings())
    private val dir = System.getProperty("dspi.shotsDir")

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val dev = LinkDevice(0, "0123456789ABCDEF", "Living room", "1.1.6", 0, 1, "online", "usb", 0)
    private val connected = LinkSessionState.Connected(hub, dev, "control")
    private val open = UsbSessionState.Open("x", UsbIds.DSPI_VID)

    /** A state of the bar: what MainActivity hands AppScaffold, and its fill by theme (null: surfaceVariant). */
    private class Case(
        val name: String,
        val strip: ConnectionStrip,
        val devices: List<DeviceRow>,
        val fill: (dark: Boolean) -> Color? = { null }
    )

    private val cases = listOf(
        Case("usb", connectionStrip(AppMode.DEVICE, open), deviceList(AppMode.DEVICE, open)),
        Case("nodevice", connectionStrip(AppMode.DEVICE, UsbSessionState.NoDevice), deviceList(AppMode.DEVICE, UsbSessionState.NoDevice)),
        Case("demo", connectionStrip(AppMode.DEMO, UsbSessionState.NoDevice), deviceList(AppMode.DEMO, UsbSessionState.NoDevice)) {
            DspiBarColors.of(it).offline.container
        },
        Case(
            "document",
            connectionStrip(AppMode.DEMO, UsbSessionState.NoDevice, document = "Living room.dspi"),
            deviceList(AppMode.DEMO, UsbSessionState.NoDevice)
        ) { DspiBarColors.of(it).offline.container },
        Case(
            "secured",
            connectionStrip(
                AppMode.REMOTE,
                UsbSessionState.NoDevice,
                connected,
                security = HubSecurity("6f1a", encrypted = true, hubKey = "k")
            ),
            deviceList(AppMode.REMOTE, UsbSessionState.NoDevice, connected, listOf(hub), linkEnabled = true)
        ) { DspiBarColors.of(it).link.container },
        Case(
            "unsecured",
            connectionStrip(
                AppMode.REMOTE,
                UsbSessionState.NoDevice,
                connected,
                security = HubSecurity("6f1a", encrypted = false, hubKey = "")
            ),
            deviceList(AppMode.REMOTE, UsbSessionState.NoDevice, connected, listOf(hub), linkEnabled = true)
        ) { StripWarningColors.of(it).container },
        Case(
            "acknowledged",
            connectionStrip(
                AppMode.REMOTE,
                UsbSessionState.NoDevice,
                connected,
                security = HubSecurity("6f1a", encrypted = false, hubKey = "", acknowledged = true)
            ),
            deviceList(AppMode.REMOTE, UsbSessionState.NoDevice, connected, listOf(hub), linkEnabled = true)
        ) { DspiBarColors.of(it).acknowledged.container }
    )

    private var case by mutableStateOf(cases.first())
    private var scheme: ColorScheme? = null

    @After fun stop() = scope.cancel()

    private fun settle() {
        repeat(20) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
        }
    }

    private fun show(dark: Boolean) {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = dark) {
                    scheme = MaterialTheme.colorScheme
                    AppScaffold(bridge, case.strip, onExitDemo = {}, onRequestPermission = {}, devices = case.devices)
                }
            }
        }
        settle()
    }

    /** The bar's height in dp, rounded. */
    private fun barHeightDp(): Int {
        val px = rule.onNodeWithTag(CONNECTION_STRIP_TAG).fetchSemanticsNode().size.height
        return Math.round(px / rule.density.density)
    }

    private fun screen(): Bitmap {
        val root = rule.activity.window.decorView
        return Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888).also { root.draw(Canvas(it)) }
    }

    /** The strip's fill as drawn: a pixel in its start padding (as ANSEC's). */
    private fun stripFill(): Int {
        val node = rule.onNodeWithTag(CONNECTION_STRIP_TAG).fetchSemanticsNode()
        val at = node.positionInWindow
        var px = 0
        rule.runOnUiThread { px = screen().getPixel(at.x.toInt() + 3, (at.y + node.size.height / 2f).toInt()) }
        return px
    }

    /** Every button on the bar is touched over at least 48dp, however thin it is drawn. */
    private fun assertTouchTargets(name: String) {
        val minPx = with(rule.density) { 48.dp.toPx() } - 1f
        val buttons = rule.onAllNodes(hasClickAction() and hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))).fetchSemanticsNodes()
        assertTrue("$name: buttons on the bar", buttons.isNotEmpty())
        for (b in buttons) {
            val touch = b.touchBoundsInRoot
            assertTrue("$name ${b.config} touch ${touch.width}x${touch.height}px", touch.height >= minPx && touch.width >= minPx)
        }
    }

    /** A text file beside the shots, when they are written. */
    private fun write(name: String, text: String) {
        if (dir.isNullOrEmpty()) return
        File(dir).apply { mkdirs() }.resolve(name).writeText(text)
    }

    private fun shoot(name: String) {
        if (dir.isNullOrEmpty()) return
        val node = rule.onNodeWithTag(CONNECTION_STRIP_TAG).fetchSemanticsNode()
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val full = screen()
            val bottom = (node.boundsInWindow.bottom + 16 * rule.density.density).toInt().coerceAtMost(full.height)
            val crop = Bitmap.createBitmap(full, 0, 0, full.width, bottom)
            out.outputStream().use { crop.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    /** Every case on one window; the heights by name. */
    private fun measure(size: String, dark: Boolean): Map<String, Int> {
        val theme = if (dark) "dark" else "light"
        show(dark)
        val heights = cases.associate { c ->
            case = c
            settle()
            shoot("bar-$size-$theme-${c.name}")
            val fill = c.fill(dark) ?: scheme!!.surfaceVariant
            assertEquals("$size $theme ${c.name} fill", fill.toArgb(), stripFill())
            assertTouchTargets("$size $theme ${c.name}")
            c.name to barHeightDp()
        }
        val report = heights.entries.joinToString("\n") { (n, h) -> "$size $theme $n ${h}dp" }
        println("WP ANBAR heights\n$report")
        write("bar-heights-$size-$theme.txt", report + "\n")
        for ((name, h) in heights) {
            // One line at most 40dp; a phone's no-device bar has its actions' line below (ANLINK)
            val max = if (size == "phone" && name == "nodevice") 40 + STRIP_BUTTON_HEIGHT.value.toInt() else 40
            assertTrue("$size $theme $name: ${h}dp", h <= max)
        }
        return heights
    }

    private fun contrast(a: Color, b: Color): Double {
        val (hi, lo) = listOf(a.luminance().toDouble(), b.luminance().toDouble()).sortedDescending()
        return (hi + 0.05) / (lo + 0.05)
    }

    /**
     * Text and icons at WCAG AA (4.5:1) on every tint, both themes; Material's tertiary is ours,
     * so its default pink shows nowhere; the tints differ from each other and from USB's fill.
     */
    @Test
    fun wpAnbarTheTintsAreOursWithAaContrast() {
        var light: ColorScheme? = null
        var dark: ColorScheme? = null
        rule.setContent {
            DspiTheme(darkTheme = false) { light = MaterialTheme.colorScheme }
            DspiTheme(darkTheme = true) { dark = MaterialTheme.colorScheme }
        }
        rule.waitForIdle()
        val ratios = StringBuilder()
        for ((isDark, s) in listOf(false to light!!, true to dark!!)) {
            val theme = if (isDark) "dark" else "light"
            val bar = DspiBarColors.of(isDark)
            val warning = StripWarningColors.of(isDark)
            val tints = mapOf(
                "link" to bar.link,
                "offline" to bar.offline,
                "acknowledged" to bar.acknowledged,
                "warning" to BarColors(warning.container, warning.content),
                "usb" to BarColors(s.surfaceVariant, s.onSurface)
            )
            for ((name, t) in tints) {
                // As drawn: a translucent ink (the theme's labels) over its fill
                val r = contrast(t.content.compositeOver(t.container), t.container)
                ratios.append("$theme $name ${"%.2f".format(r)}\n")
                assertTrue("$theme $name $r", r >= 4.5)
            }
            assertEquals("$theme: no default tertiary", bar.link.container, s.tertiaryContainer)
            assertEquals("$theme: no default tertiary", bar.link.content, s.onTertiaryContainer)
            assertEquals("$theme: no default tertiary", (if (isDark) DspiColors.Dark else DspiColors.Light).accent, s.tertiary)
            val fills = tints.values.map { it.container }
            assertEquals("$theme: five distinct fills", fills.size, fills.toSet().size)
        }
        println("WP ANBAR contrast\n$ratios")
        write("bar-contrast.txt", ratios.toString())
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnbarPhoneLight() {
        measure("phone", dark = false)
    }

    @Test
    @Config(qualifiers = "w400dp-h860dp-port-xhdpi")
    fun wpAnbarPhoneDark() {
        measure("phone", dark = true)
    }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-land-mdpi")
    fun wpAnbarWideLight() {
        measure("wide", dark = false)
    }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-land-mdpi")
    fun wpAnbarWideDark() {
        measure("wide", dark = true)
    }
}
