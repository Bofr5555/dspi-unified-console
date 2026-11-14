package se.bofr.dspi.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.BootloaderDrive
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.UnimplementedDspiCore

/** WP AN28: what would leave the demo device stops at [DemoCore]. */
class DemoCoreAn28Test {
    /** Throws on anything else, so a call that reaches past the guard fails the test. */
    private class OnlyTheDemoDevice : UnimplementedDspiCore() {
        var connected = false
        var opens = 0

        override fun isConnected() = connected

        override fun openDemo() {
            opens++
            connected = true
        }
    }

    private val inner = OnlyTheDemoDevice()
    private val core = DemoCore(inner)

    @Test
    fun an28TheOnlyDeviceIsTheDemoDeviceAndItOpensOnce() {
        assertEquals(listOf(DEMO_DEVICE_SERIAL), core.scanDevices().map { it.serial })
        assertEquals(0L, core.inaccessibleDevices())
        assertFalse(core.isConnected())
        core.selectDevice(DEMO_DEVICE_SERIAL)
        core.selectDevice(DEMO_DEVICE_SERIAL)
        assertTrue(core.isConnected())
        assertEquals(1, inner.opens)
        assertThrows(DspiException.Disconnected::class.java) { core.selectDevice("E6614103") }
    }

    @Test
    fun an28UsbFlashingAndLinkAreRefusedBeforeTheCore() {
        val drive = BootloaderDrive("/mnt/RP2350", 1, "")
        val board = FirmwareGuardBoard(1200, FirmwareGuardBoardSource.values().first())
        val refused = listOf<() -> Unit>(
            { core.openFd(3, "E6614103", 0x2E8B) },
            { core.flashToDrive(listOf(0), drive) },
            { core.flashToDriveGuarded(listOf(0), drive, board, "yes") },
            { core.flashConfirmed(listOf(0), drive, board, "yes", null) }, // -- WP P1AN1 --
            { core.enterBootloaderConfirmed(null) },
            { core.firmwareGuardBoardInBootloader() },
            { core.linkConnect("ws://hub", "demo", "1", "", "") },
            { core.linkPair("1234") },
            { core.linkSelectDevice(0, "E6614103") },
            { core.linkFetchInfo("hub", 80) }
        )
        refused.forEach { call -> assertThrows(DspiException.Unsupported::class.java) { call() } }
        assertTrue(core.bootloaderDrives().isEmpty())
        assertEquals(0L, core.bootloaderCounts().rp2350)
        assertTrue(core.linkDevices().isEmpty())
        core.pollHotplug()
        core.linkRetryNow()
        core.linkSetUrl("ws://hub")
    }

    /** A new way out to hardware fails here until it is classified, instead of passing to the core by delegation.
     *  `openDemoAlink` (SW1) opens only the core's fake device with an audio-link model, like `openDemo`, so it is
     *  classified as safe to delegate. */
    @Test
    fun an28EveryCallThatLooksLikeAWayOutIsListed() {
        val names = DspiCoreApi::class.java.methods.map { it.name }.toSet()
        val wayOut = Regex(
            "^(open|flash|scan|select|poll|enter|bootloader|linkConnect|linkPair|linkSelect|linkDevices|linkRetry|linkSetUrl|linkFetch)|InBootloader"
        )
        val guarded = setOf(
            "openFd", "openDemo", "openDemoAlink", "scanDevices", "selectDevice", "pollHotplug", "bootloaderCounts",
            "bootloaderDrives",
            "flashToDrive", "flashToDriveGuarded", "firmwareGuardBoardInBootloader", "linkConnect", "linkPair", "linkDevices",
            "linkSelectDevice", "linkRetryNow", "linkSetUrl", "linkFetchInfo",
            // -- WP P1AN1 -- P1OF's `open_document` and the alink USB-direct demo open only the core's fake device;
            // P1TK's confirmed bootloader and flash calls are refused too (WP P1TZ: the bootloader's only form)
            "openDocument", "openDemoUsbDirect", "flashConfirmed", "enterBootloaderConfirmed"
        )
        assertEquals(guarded, names.filter { wayOut.containsMatchIn(it) }.toSet())
    }
}
