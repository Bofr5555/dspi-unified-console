package se.bofr.dspi.bridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.InstallerPin
import se.bofr.dspi.core.LimitsApply
import se.bofr.dspi.core.LimitsStatus
import se.bofr.dspi.core.LockState
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP HUBADMIN in the bridge, on the fake's hub model: the Installer calls go to
 * the hub's copy when `installer_target().copy` is "hub" and to this console's
 * own as before otherwise; the hub refuses below Admin; a role's mode is pinned
 * on the hub; the hub's event makes the bridge read the hub's copy again and
 * take the event's lock.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class InstallerHubAdminTest {
    /** The fake with this console's own copy recorded (the fake does not model it). */
    private class Core : FakeDspiCore() {
        private fun lockOf(pinSet: Boolean) = LockState(
            serial = serial, pinSet = pinSet, unlocked = true, locked = false, installerOpen = true, covers = emptyList(),
            allowedWrites = emptyList(), wrongTries = 0, retryAfterMs = 0, recovered = false, note = "", contact = "",
            scopeNoticeKey = "", detail = "", heldBy = "", hubPinnedMode = ""
        )

        var consoleProfile = ""

        override fun installerLockState(): LockState = if (hubCopy == "hub") super.installerLockState() else lockOf(false)

        override fun installerLockRefresh(): LockState = if (hubCopy == "hub") super.installerLockRefresh() else lockOf(false)

        override fun limitsStatus() = LimitsStatus(consoleProfile.isNotEmpty(), serial, "none", "", "", emptyList())

        override fun limitsProfileDocument(): String = consoleProfile

        override fun limitsApplyProfile(document: String): LimitsApply {
            calls += "limitsApplyProfile"
            consoleProfile = document
            return LimitsApply(listOf("applied"), emptyList(), emptyList())
        }

        override fun limitsClear() {
            calls += "limitsClear"
            consoleProfile = ""
        }

        override fun installerPinSet(pin: String): InstallerPin {
            calls += "installerPinSet"
            return InstallerPin("CONSO-LECOD-EAAAA-BBBBB", lockOf(true))
        }

        override fun installerPinClear(): LockState {
            calls += "installerPinClear"
            return lockOf(false)
        }

        override fun installerLockCover(pages: List<String>): LockState {
            calls += "installerLockCover($pages)"
            return lockOf(true)
        }
    }

    private val hubCalls = listOf("hubLimitsSet", "hubLimitsClear", "hubPinSet", "hubPinClear", "hubModePin")
    private val consoleCalls = listOf("limitsApplyProfile", "limitsClear", "installerPinSet", "installerPinClear", "installerLockCover")

    private fun FakeDspiCore.since(mark: Int) = calls.drop(mark).map { it.substringBefore('(') }

    @Test
    fun wpHubadminOverUsbTheInstallerEditsThisConsolesCopyAsBefore() = runTest {
        val core = Core()
        val b = newBridge(core)
        attach(b, core)
        val i = b.installer
        runCurrent()
        assertEquals(Installer.COPY_CONSOLE, i.target.value?.copy)
        val mark = core.calls.size
        assertTrue(i.applyLimits(LimitsDraft(masterVolumeMaxDb = -10f)).isSuccess)
        assertEquals("CONSO-LECOD-EAAAA-BBBBB", i.setPin("1234").getOrThrow())
        assertTrue(i.cover(listOf("front")).isSuccess)
        assertTrue(i.clearPin().isSuccess)
        assertTrue(i.clearLimits().isSuccess)
        runCurrent()
        val made = core.since(mark)
        assertTrue(made.toString(), made.containsAll(consoleCalls))
        assertTrue("nothing went to the hub: $made", made.none { it in hubCalls || it == "hubLimitsGet" })
        assertEquals(null, i.hub.value)
    }

    @Test
    fun wpHubadminThroughAHubTheInstallerEditsTheHubsCopy() = runTest {
        val core = Core().apply { hubCopy = "hub" }
        val b = newBridge(core)
        attach(b, core)
        val i = b.installer
        runCurrent()
        assertEquals(Installer.COPY_HUB, i.target.value?.copy)
        assertTrue(i.target.value!!.editable)
        assertNotNull("the hub's copy read", i.hub.value)
        val mark = core.calls.size
        core.hubHeldNext = listOf("OUT2 gain held to -6 dB")
        val report = i.applyLimits(LimitsDraft(masterVolumeMaxDb = -10f).withOutput(OutputLimits(1, maxGainDb = -6f))).getOrThrow()
        assertEquals("what the hub held at once is the report", listOf("OUT2 gain held to -6 dB"), report.held)
        assertEquals(-10f, i.profile.value?.masterVolumeMaxDb)
        assertEquals("the hub's document", -10f, LimitsDraft.fromDocument(core.hubProfile)?.masterVolumeMaxDb)
        // The lock covers the profile's lockedPages on the hub, the rest of the profile kept
        i.setPin("4321").getOrThrow()
        i.cover(listOf("front")).getOrThrow()
        assertEquals(listOf("front"), LimitsDraft.fromDocument(core.hubProfile)?.lockedPages)
        assertEquals(-10f, LimitsDraft.fromDocument(core.hubProfile)?.masterVolumeMaxDb)
        assertEquals(listOf("front"), i.profile.value?.lockedPages)
        assertEquals(Installer.HELD_BY_HUB, i.lock.value?.heldBy)
        assertTrue(i.hub.value!!.pinSet)
        i.clearPin().getOrThrow()
        assertFalse(i.hub.value!!.pinSet)
        i.clearLimits().getOrThrow()
        assertEquals(null, i.profile.value)
        runCurrent()
        val made = core.since(mark)
        assertTrue(made.toString(), made.containsAll(listOf("hubLimitsSet", "hubPinSet", "hubPinClear", "hubLimitsClear")))
        assertTrue("nothing went to this console's copy: $made", made.none { it in consoleCalls })
    }

    @Test
    fun wpHubadminBelowAdminTheHubsCopyIsReadOnlyAndRefused() = runTest {
        val core = Core().apply {
            hubCopy = "hub"
            hubRole = "control"
        }
        val b = newBridge(core)
        attach(b, core)
        val i = b.installer
        runCurrent()
        val t = i.target.value!!
        assertFalse(t.editable)
        assertEquals("installer.hub_role", t.readOnlyKey)
        assertEquals("installer.hub_copy", t.noticeKey)
        assertTrue(i.applyLimits(LimitsDraft(masterVolumeMaxDb = -10f)).isFailure)
        assertTrue(i.setPin("1234").isFailure)
        assertTrue(i.pinMode("viewer", "simple").isFailure)
        assertEquals("", core.hubProfile)
        // Control sees its own role's pinned mode only
        assertEquals(listOf("control"), i.hub.value!!.pinnedModes.map { it.role })
    }

    @Test
    fun wpHubadminAModeIsPinnedOnTheHubAndRefusedOffAHub() = runTest {
        val core = Core().apply { hubCopy = "hub" }
        val b = newBridge(core)
        attach(b, core)
        val i = b.installer
        runCurrent()
        assertEquals(listOf("viewer", "control", "admin"), i.hub.value!!.pinnedModes.map { it.role })
        val s = i.pinMode("viewer", Installer.SIMPLE).getOrThrow()
        assertEquals(Installer.SIMPLE, s.pinnedModes.first { it.role == "viewer" }.mode)
        assertEquals(s, i.hub.value)
        assertTrue(core.calls.contains("hubModePin(viewer, simple)"))
        assertTrue("a mode the hub does not know", i.pinMode("viewer", "expert").isFailure)
        i.pinMode("viewer", "").getOrThrow()
        assertEquals("", i.hub.value!!.pinnedModes.first { it.role == "viewer" }.mode)
        core.hubCopy = "console"
        assertTrue("off a hub: nothing to pin", i.pinMode("viewer", Installer.SIMPLE).isFailure)
    }

    @Test
    fun wpHubadminTheHubsEventReadsTheCopyAgainAndTakesTheEventsLock() = runTest {
        val core = Core().apply { hubCopy = "hub" }
        val b = newBridge(core)
        attach(b, core)
        val i = b.installer
        runCurrent()
        assertNotNull("the listener is registered", core.hubAdminListener)
        assertEquals(null, i.profile.value)
        // Another Admin set a profile and a PIN on the hub
        core.hubSetProfileRemotely(LimitsDraft(masterVolumeMaxDb = -12f, lockedPages = listOf("front")).toDocument(core.serial))
        core.hubRole = "admin"
        val reads = core.callCount("hubLimitsGet")
        core.hubEvent("limits.changed")
        runCurrent()
        assertEquals(reads + 1, core.callCount("hubLimitsGet"))
        assertEquals(-12f, i.profile.value?.masterVolumeMaxDb)
        assertEquals(core.hubLock(), i.lock.value)
        // mode.changed: the pinned modes read again
        core.hubPinned["control"] = Installer.ADVANCED
        core.hubEvent("mode.changed", role = "control", mode = Installer.ADVANCED)
        runCurrent()
        assertEquals(Installer.ADVANCED, i.hub.value!!.pinnedModes.first { it.role == "control" }.mode)
    }
}
