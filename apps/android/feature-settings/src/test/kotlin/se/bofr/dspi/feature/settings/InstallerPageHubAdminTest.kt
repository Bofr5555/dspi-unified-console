package se.bofr.dspi.feature.settings

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.Installer
import se.bofr.dspi.bridge.LimitsDraft
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP HUBADMIN on the Installer page's model (apart from Compose), on the
 * fake's hub model: the page says which copy it edits with the core's row,
 * is read-only below Admin with the `readOnlyKey` row and sends nothing, shows
 * the hub's recovery code once with `installer_lock.hub_recovery`, and pins a
 * role's mode on the hub.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class InstallerPageHubAdminTest {
    /** The fake with the core's notice table stood in by "title/reason/action" of the key. */
    private class Core : FakeDspiCore() {
        override fun noticeForKey(key: String, detail: String) = Notice(key, NoticeKind.INFO, "T:$key", "R:$key", "A:$key", detail)
    }

    private suspend fun TestScope.page(core: Core): Pair<DspiBridgeImpl, InstallerPageModel> {
        val d = StandardTestDispatcher(testScheduler)
        val bridge = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        val model = InstallerPageModel(bridge, backgroundScope)
        model.load()
        runCurrent()
        return bridge to model
    }

    @Test
    fun wpHubadminThePageSaysWhichCopyItEdits() = runTest {
        val usb = Core()
        val (_, m1) = page(usb)
        assertEquals("T:installer.console_copy: R:installer.console_copy", m1.copyLine.value)
        assertTrue(m1.editable)
        assertEquals("", m1.readOnlyLine.value)
        val hub = Core().apply { hubCopy = "hub" }
        val (_, m2) = page(hub)
        assertEquals("T:installer.hub_copy: R:installer.hub_copy", m2.copyLine.value)
        assertTrue(m2.editable)
    }

    @Test
    fun wpHubadminBelowAdminThePageIsReadOnlyAndSendsNothing() = runTest {
        val core = Core().apply {
            hubCopy = "hub"
            hubRole = "control"
        }
        val (_, m) = page(core)
        assertFalse(m.editable)
        assertEquals("T:installer.hub_role: R:installer.hub_role A:installer.hub_role", m.readOnlyLine.value)
        val mark = core.calls.size
        m.apply(LimitsDraft(masterVolumeMaxDb = -10f))
        m.setPin("1234", "1234")
        m.clearPin()
        m.clearLimits()
        m.setCovered("front", true)
        m.pinMode("viewer", Installer.SIMPLE)
        runCurrent()
        val sent = core.calls.drop(mark).filter { it.startsWith("hub") && !it.startsWith("hubLimitsGet") }
        assertEquals("nothing sent while read-only", emptyList<String>(), sent)
        assertTrue(m.outcome.value.isError)
        assertEquals(m.readOnlyLine.value, m.outcome.value.text)
        assertEquals("", m.recovery.value)
        // A hub without limits: nothing to edit either, with its row
        core.hubCopy = "none"
        m.load()
        runCurrent()
        assertFalse(m.editable)
        assertEquals("T:dlink.no_limits: R:dlink.no_limits", m.copyLine.value)
    }

    @Test
    fun wpHubadminTheHubsRecoveryCodeIsShownOnce() = runTest {
        val core = Core().apply { hubCopy = "hub" }
        val (bridge, m) = page(core)
        m.setPin("1234", "1234")
        runCurrent()
        assertTrue(core.calls.contains("hubPinSet"))
        assertEquals("HUBRC-00001-AAAAA-BBBBB", m.recovery.value)
        assertEquals("the hub's row", "R:installer_lock.hub_recovery A:installer_lock.hub_recovery", m.recoveryNote.value)
        assertTrue(core.hubPinSet)
        m.recoveryNoted()
        assertEquals("", m.recovery.value)
        // Neither a re-read nor the hub's event brings it back
        core.hubEvent("limits.changed")
        bridge.installer.refresh(hub = true)
        m.load()
        runCurrent()
        assertEquals("", m.recovery.value)
        assertEquals("", m.recoveryNote.value)
    }

    @Test
    fun wpHubadminARolesModeIsPinnedFromThePage() = runTest {
        val core = Core().apply { hubCopy = "hub" }
        val (bridge, m) = page(core)
        assertEquals(Installer.HUB_ROLES, m.hub.value!!.pinnedModes.map { it.role })
        m.pinMode("control", Installer.INSTALLER)
        runCurrent()
        assertEquals(Installer.INSTALLER, bridge.installer.hub.value!!.pinnedModes.first { it.role == "control" }.mode)
        assertEquals("Control: Installer.", m.outcome.value.text)
        assertFalse(m.outcome.value.isError)
        assertEquals(listOf("None", "Simple", "Advanced", "Installer"), Installer.PINNABLE_MODES.map(InstallerPageModel::modeTitle))
        // The lock's cover on the hub is the profile's lockedPages
        m.setCovered("front", true)
        runCurrent()
        assertEquals(listOf("front"), m.covered())
        assertEquals(listOf("front"), LimitsDraft.fromDocument(core.hubProfile)?.lockedPages)
    }
}
