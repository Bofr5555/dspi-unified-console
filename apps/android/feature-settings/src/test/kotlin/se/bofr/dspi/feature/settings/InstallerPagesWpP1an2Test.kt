package se.bofr.dspi.feature.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.LockState
import se.bofr.dspi.fake.demoDspState

/** WP P1AN2: the Installer and power-on pages' rules apart from Compose. */
class InstallerPagesWpP1an2Test {
    private fun lock(pinSet: Boolean = false, unlocked: Boolean = false, covers: List<String> = emptyList(), heldBy: String = "") =
        LockState(
            serial = "S",
            pinSet = pinSet,
            unlocked = unlocked,
            locked = pinSet && covers.isNotEmpty() && !unlocked,
            installerOpen = !pinSet || unlocked,
            covers = covers,
            allowedWrites = emptyList(),
            wrongTries = 0,
            retryAfterMs = 0,
            recovered = false,
            note = "",
            contact = "",
            scopeNoticeKey = "",
            detail = "",
            heldBy = heldBy,
            hubPinnedMode = ""
        )

    @Test
    fun wpP1an2TheInstallerPagesShowOnlyInTheInstallerModeAndLockedShowsTheNote() {
        val a = SettingsAvailability(connected = true, platformName = "RP2350")
        assertFalse(SettingsCategory.INSTALLER.isAvailable(a))
        assertFalse(SettingsCategory.POWER_ON.isAvailable(a))
        assertTrue(SettingsCategory.INSTALLER.isAvailable(a.copy(installerMode = true)))
        assertTrue(SettingsCategory.POWER_ON.isAvailable(a.copy(installerMode = true)))
        val locked = a.copy(lockedUser = true)
        assertEquals(
            listOf(SettingsCategory.GENERAL, SettingsCategory.INSTALLER),
            SettingsCategory.entries.filter { it.isAvailable(locked) }
        )
    }

    @Test
    fun wpP1an2TheLockLineSaysNothingIsLockedWithNoPin() {
        assertTrue(InstallerPageModel.lockLine(lock()).startsWith("No installer PIN is set"))
        assertTrue(InstallerPageModel.lockLine(lock(pinSet = true, unlocked = true)).contains("Unlocked on this console"))
        assertTrue(InstallerPageModel.lockLine(lock(pinSet = true, covers = listOf("front"))).contains("locked"))
        assertTrue(InstallerPageModel.lockLine(lock(pinSet = true, heldBy = "hub")).startsWith("The hub holds"))
        assertEquals("", InstallerPageModel.unprotectedLine(emptyList()))
        assertEquals("OUT1, OUT3 are above -10 dB gain with no limiter.", InstallerPageModel.unprotectedLine(listOf(0, 2)))
    }

    @Test
    fun wpP1an2TestPowerOnLoadsTheSlotTheDeviceStartsWith() {
        val s = demoDspState().copy(presetStartupMode = GlobalSettingsDraft.STARTUP_SPECIFIED, presetDefaultSlot = 4, activePresetSlot = 1)
        assertEquals(4, PowerOn.startupSlot(s))
        assertEquals(1, PowerOn.startupSlot(s.copy(presetStartupMode = GlobalSettingsDraft.STARTUP_LAST_ACTIVE)))
    }
}
