package se.bofr.dspi.app

import se.bofr.dspi.core.BootloaderCounts
import se.bofr.dspi.core.BootloaderDrive
import se.bofr.dspi.core.DeviceInfo
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkHello
import se.bofr.dspi.core.LinkInfo

/** The serial the core's demo device reports (`dspi_transport::fake`). */
internal const val DEMO_DEVICE_SERIAL = "DEMO0000DSPI0001"

/**
 * WP AN28: the real core on its demo device (`open_demo`), with every way out to
 * real hardware or a network closed.
 *
 * The bridge finds a device the usual way: [scanDevices] lists the demo device and
 * [selectDevice] opens it, so the open runs on the bridge's FFI thread. Everything
 * else goes to the core, so a screen sees the same behaviour it sees on firmware.
 * What would reach past the demo device is refused here, whatever the UI asks.
 */
internal class DemoCore(private val core: DspiCoreApi) : DspiCoreApi by core {
    override fun scanDevices(): List<DeviceInfo> = listOf(DeviceInfo(DEMO_DEVICE_SERIAL, 0))

    override fun inaccessibleDevices(): Long = 0

    override fun selectDevice(serial: String) {
        if (serial != DEMO_DEVICE_SERIAL) throw DspiException.Disconnected()
        if (!core.isConnected()) core.openDemo()
    }

    // USB
    override fun openFd(fd: Int, serial: String, vid: Int): Unit = throw DspiException.Unsupported()

    override fun pollHotplug() = Unit

    // A real board in BOOTSEL is a drive like any other; the demo device has no bootloader
    // (enterBootloaderConfirmed below; WP P1TZ retired the form without a token).

    override fun bootloaderCounts() = BootloaderCounts(0, 0)

    override fun bootloaderDrives(): List<BootloaderDrive> = emptyList()

    override fun flashToDrive(image: List<Int>, drive: BootloaderDrive): Unit = throw DspiException.Unsupported()

    override fun firmwareGuardBoardInBootloader(): FirmwareGuardBoard = throw DspiException.Unsupported()

    override fun flashToDriveGuarded(image: List<Int>, drive: BootloaderDrive, board: FirmwareGuardBoard, typedConfirmation: String): Unit =
        throw DspiException.Unsupported()

    // -- WP P1AN1 -- P1TK's token-taking forms of the same ways out (found missing by an28's test)
    override fun flashConfirmed(
        image: List<Int>,
        drive: BootloaderDrive,
        board: FirmwareGuardBoard,
        typedConfirmation: String,
        token: String?
    ): Unit = throw DspiException.Unsupported()

    override fun enterBootloaderConfirmed(token: String?): Unit = throw DspiException.Unsupported()

    // DSPi Link and its hubs
    override fun linkConnect(url: String, clientName: String, appVersion: String, token: String, expectedHubId: String): LinkHello =
        throw DspiException.Unsupported()

    override fun linkPair(pin: String): String = throw DspiException.Unsupported()

    override fun linkDevices(): List<LinkDevice> = emptyList()

    override fun linkSelectDevice(handle: Long, serial: String): Unit = throw DspiException.Unsupported()

    override fun linkRetryNow() = Unit

    override fun linkSetUrl(url: String) = Unit

    override fun linkFetchInfo(host: String, port: Int): LinkInfo = throw DspiException.Unsupported()
}
