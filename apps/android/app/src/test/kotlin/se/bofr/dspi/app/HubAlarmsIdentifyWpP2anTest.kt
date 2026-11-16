package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import java.util.Collections
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.HubAlarm
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.fake.FakeHubDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.alert.AlertStyle
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP P2AN in the app (Robolectric, the bridge's own FFI thread, the fake's
 * hub model): Identify sits in the strip's overflow on a phone (no new
 * button on the bar) and blinks the device through the hub; a hub without the
 * cap has no Identify; an alarm the hub raises shows once as the core's
 * notice with Acknowledge, which acknowledges it on the hub; a Viewer gets no
 * Acknowledge. Compose state written on main (ANDFU).
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h860dp-port-xhdpi")
class HubAlarmsIdentifyWpP2anTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    @After fun stop() = scope.cancel()

    private fun pumpUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(5)
        }
    }

    private fun textShown(t: String) =
        rule.onAllNodes(hasText(t, substring = true), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun scaffold(core: FakeHubDspiCore): DspiBridgeImpl {
        val bridge = DspiBridgeImpl(core, scope)
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        pumpUntil("connected") { bridge.status.value.connected }
        val store = InMemoryAppSettingsStore(AppSettings())
        val modes = InMemoryUiModeStore().apply { save("", "advanced", everyDevice = true) }
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                DspiTheme(darkTheme = false) {
                    AppScaffold(bridge, ConnectionStrip.Usb, onExitDemo = {}, onRequestPermission = {}, modeStore = modes)
                }
            }
        }
        pumpUntil("caps") { bridge.hubServer.caps.value.onHub == core.onHub }
        return bridge
    }

    private fun openMenu() {
        rule.onNodeWithTag(STRIP_MENU_TAG).performClick()
        pumpUntil("menu") { textShown("Snapshots") }
    }

    @Test
    fun wpP2anIdentifyIsInTheOverflowAndBlinksThroughTheHub() {
        val core = FakeHubDspiCore()
        val offMain = Collections.synchronizedList(mutableListOf<String>())
        val observer = Snapshot.registerGlobalWriteObserver {
            if (Looper.myLooper() != Looper.getMainLooper()) offMain += Thread.currentThread().name
        }
        try {
            scaffold(core)
            pumpUntil("writes") { rule.onAllNodes(hasTestTag(STRIP_MENU_TAG)).fetchSemanticsNodes().isNotEmpty() }
            openMenu()
            pumpUntil("identify entry") { textShown(IDENTIFY_ENTRY) }
            rule.onNode(hasText(IDENTIFY_ENTRY)).performClick()
            pumpUntil("identified") { core.identified.isNotEmpty() }
            assertEquals(listOf(IdentifyAction.IDENTIFY_SECONDS to null), core.identified.toList())
            assertTrue(core.calls.contains("hubIdentify(10)"))
        } finally {
            observer.dispose()
        }
        assertEquals("Compose state written off the main thread", emptyList<String>(), offMain.toList())
    }

    @Test
    fun wpP2anAHubWithoutTheCapHasNoIdentify() {
        val core = FakeHubDspiCore(hubCaps = setOf("events"))
        scaffold(core)
        openMenu()
        rule.mainClock.advanceTimeBy(300)
        shadowOf(Looper.getMainLooper()).idle()
        assertFalse(textShown(IDENTIFY_ENTRY))
    }

    @Test
    fun wpP2anARaisedAlarmShowsOnceAndIsAcknowledgedOnTheHub() {
        val core = FakeHubDspiCore()
        // -- WP ANDFIX1 -- the alert and the notice line are written on main (the alarm's notice is read on the FFI thread)
        val offMain = Collections.synchronizedList(mutableListOf<String>())
        val observer = Snapshot.registerGlobalWriteObserver {
            if (Looper.myLooper() != Looper.getMainLooper()) offMain += Thread.currentThread().name
        }
        try {
            alarmShownOnceAndAcknowledged(core)
        } finally {
            observer.dispose()
        }
        assertEquals("Compose state written off the main thread", emptyList<String>(), offMain.toList())
    }

    private fun alarmShownOnceAndAcknowledged(core: FakeHubDspiCore) {
        val bridge = scaffold(core)
        val a = core.hubRaiseAlarm("device_restarted", severity = "critical", message = "watchdog")
        pumpUntil("alarm") { bridge.hubServer.alarms.value.any { it.id == a.id } }
        pumpUntil("notice") { textShown("Alarm device_restarted") && textShown(HubAlarmNotices.ACKNOWLEDGE) }
        assertTrue(textShown("watchdog"))
        rule.onNode(hasText(HubAlarmNotices.ACKNOWLEDGE)).performClick()
        pumpUntil("acknowledged") { core.hubAlarmList.first().state == "cleared" }
        pumpUntil("gone") { !textShown("Alarm device_restarted") }
        assertTrue(core.calls.contains("hubAlarmAck:write"))
        // Raised again: not shown again
        core.serverListener?.onHubServerEvent(
            se.bofr.dspi.core.HubServerEvent("alarm.changed", core.serial, null, null, "", "", a.copy(count = 2), null, null, null)
        )
        rule.mainClock.advanceTimeBy(300)
        shadowOf(Looper.getMainLooper()).idle()
        assertFalse(textShown("Alarm device_restarted"))
    }

    @Test
    fun wpP2anAViewerSeesTheAlarmWithoutAcknowledge() {
        val alarm = HubAlarm(
            "device:1", "device", "temperature_high", "S", "S", "warning", "54 °C", "raised", "2026-11-15T10:00:00Z", "", "", 1, "",
            "alarm.temperature_high"
        )
        val notice = Notice("alarm.temperature_high", NoticeKind.WARNING, "Running hot", "Since 10:00", "Give it air.", "54 °C")
        val viewer = HubAlarmNotices.alertOf(alarm, notice, canAcknowledge = false, onAcknowledge = {}, onLater = {})
        assertEquals("Running hot", viewer.messageText)
        assertEquals("Since 10:00\n\nGive it air.\n\n54 °C", viewer.informativeText)
        assertEquals(listOf(HubAlarmNotices.LATER), viewer.buttons.map { it.title })
        assertEquals(AlertStyle.WARNING, viewer.style)
        val control = HubAlarmNotices.alertOf(alarm.copy(severity = "critical"), null, canAcknowledge = true, onAcknowledge = {
        }, onLater = {})
        assertEquals("Alarm", control.messageText)
        assertEquals("54 °C", control.informativeText)
        assertEquals(listOf(HubAlarmNotices.ACKNOWLEDGE, HubAlarmNotices.LATER), control.buttons.map { it.title })
        assertEquals(AlertStyle.CRITICAL, control.style)
    }
}
