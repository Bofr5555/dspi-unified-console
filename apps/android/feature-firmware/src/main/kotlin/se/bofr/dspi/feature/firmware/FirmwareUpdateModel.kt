// The @State of macOS `FirmwareUpdateView` (confirmed, rebootRequested,
// lastSeenChip) plus its `start()` / `resetRun()`, lifted out of the
// composable so it survives leaving the screen mid-install and is JVM-tested.
// Android additions (owner requirements, UPSTREAM-DEVIATIONS "FIRMWARE"): the
// image source (bundled official release or a local .uf2 — first class) and
// the write path (PICOBOOT or the board's drive through SAF).
package se.bofr.dspi.feature.firmware

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import se.bofr.dspi.bridge.FirmwareLabel

/** -- review PG-D2 -- Why the bundled release is not offered to the alink sender. */
const val SENDER_BUNDLED_TEXT =
    "This is the audio link sender. The bundled firmware is for the DSP only: choose the sender's own image (.uf2)."

/** What will be written. */
sealed class FirmwareSource {
    /** The official release bundled with the app. */
    data object Bundled : FirmwareSource()

    /** A .uf2 the user chose on the phone. */
    class Local(val image: FirmwareImage) : FirmwareSource()
}

class FirmwareUpdateModel(
    val installer: FirmwareInstaller,
    private val bundled: (BootloaderBoard.Chip) -> FirmwareImage,
    private val picobootWriter: FirmwareWriter,
    private val driveWriter: (Uf2Volume?) -> FirmwareWriter = { DriveFirmwareWriter(it) }
) {
    /** The user committed to the update (macOS `confirmed`). */
    var confirmed by mutableStateOf(false)
        private set

    var rebootRequested by mutableStateOf(false)
        private set

    /** The Update confirmation is open (kept here so it survives rotation, E9). */
    var confirmingUpdate by mutableStateOf(false)

    /** WP AN18: what Export Configuration... came to, shown until dismissed. */
    var exportMessage by mutableStateOf<String?>(null)

    /** Chip of the board most recently seen, so the writing card can name it. */
    var lastSeenChip by mutableStateOf<BootloaderBoard.Chip?>(null)

    var source by mutableStateOf<FirmwareSource>(FirmwareSource.Bundled)
        private set

    /**
     * -- WP SECFIX -- The chosen local file is not a bundled image (the core's manifest):
     * "Not a verified DSPi Console image" and why, shown with it. Null for the bundled
     * release, or a file that is one byte for byte. Never a refusal.
     */
    var localLabel by mutableStateOf<FirmwareLabel?>(null)
        private set

    /** Why the last chosen file was refused, null if it wasn't. */
    var localFileError by mutableStateOf<String?>(null)
        private set

    var useDrive by mutableStateOf(false)
        private set

    var driveVolume by mutableStateOf<Uf2Volume?>(null)
        private set

    /** Settings can't change once committed. */
    val canChangeSetup: Boolean get() = !confirmed

    val bundledVersion: String get() = FirmwareVersion.expected?.description ?: "unknown"

    /** What the header and cards call the firmware being written. */
    val imageLabel: String
        get() = when (val s = source) {
            FirmwareSource.Bundled -> bundledVersion
            is FirmwareSource.Local -> s.image.name
        }

    fun chooseBundled() {
        if (!canChangeSetup) return
        if (apply(FirmwareSource.Bundled, useDrive, driveVolume)) {
            localFileError = null
            localLabel = null // -- WP SECFIX --
        }
    }

    /** [label]: [se.bofr.dspi.bridge.CoreTools.firmwareLabel] of [image] (-- WP SECFIX --). */
    fun chooseLocal(image: FirmwareImage, label: FirmwareLabel? = FirmwareLabel.FALLBACK) {
        if (!canChangeSetup) return
        if (apply(FirmwareSource.Local(image), useDrive, driveVolume)) {
            localFileError = null
            localLabel = label
        }
    }

    fun localFileRefused(reason: String) {
        localFileError = reason
    }

    fun usePicoboot() {
        if (!canChangeSetup) return
        apply(source, false, driveVolume)
    }

    /**
     * Write through the board's drive; [volume] is the drive just picked.
     * Before committing, any time; after committing only while the installer
     * waits for the drive (`WaitingForVolume`, the card's "Choose Drive..."),
     * which is how a committed drive install proceeds. Returns whether it applied.
     */
    fun useDrive(volume: Uf2Volume? = null): Boolean {
        if (confirmed && installer.state.value !is FirmwareInstallState.WaitingForVolume) return false
        return apply(source, true, volume ?: driveVolume)
    }

    /** Hands the setup to the installer; nothing changes while it is installing. */
    private fun apply(source: FirmwareSource, drive: Boolean, volume: Uf2Volume?): Boolean {
        val provider: (BootloaderBoard.Chip) -> FirmwareImage = when (source) {
            FirmwareSource.Bundled -> bundled
            is FirmwareSource.Local -> { _ -> source.image }
        }
        if (!installer.configure(writer = if (drive) driveWriter(volume) else picobootWriter, imageProvider = provider)) return false
        this.source = source
        useDrive = drive
        driveVolume = volume
        return true
    }

    /**
     * -- WP PGAND -- The Update button: with a device connected, the update
     * check runs first, on the board as it runs now ([chip] is its chip), and
     * the confirmation shows its answer ([FirmwareInstaller.entryCheck]).
     */
    fun requestUpdate(deviceConnected: Boolean, chip: BootloaderBoard.Chip?, alinkSender: Boolean = false) {
        if (!canChangeSetup) return
        if (deviceConnected && !rebootRequested && chip != null) {
            // -- review PG-D2 -- the bundled images are the DSP's: none is offered to the
            // alink sender (as the Qt console); its own image is a local .uf2
            if (alinkSender && source == FirmwareSource.Bundled) {
                installer.refuseBeforeEntry(SENDER_BUNDLED_TEXT)
            } else {
                installer.checkBeforeEntry(chip)
            }
        }
        confirmingUpdate = true
    }

    /** -- WP PGAND -- The confirmation was dismissed: nothing is kept of its check. */
    fun dismissUpdate() {
        confirmingUpdate = false
        if (!confirmed) installer.cancelEntry()
    }

    /**
     * macOS `start()`: commits to the update. The installer writes as soon as a
     * board is ready; [enterBootloader] restarts a connected device into
     * BOOTSEL (called once, with the user's ENTER_BOOTLOADER confirmation).
     * WP PGAND: with a device connected, only once the update check before the
     * entry cleared the image ([FirmwareInstaller.enterWhenClear]): a board
     * without BOOTSEL, or one whose phrase was not typed, is not rebooted and
     * nothing is committed. Returns whether the update was committed.
     */
    fun start(deviceConnected: Boolean, enterBootloader: () -> Unit): Boolean {
        if (!deviceConnected || rebootRequested) {
            confirmed = true
            installer.installWhenReady()
            return true
        }
        if (!installer.enterWhenClear(enterBootloader)) return false
        confirmed = true
        rebootRequested = true
        return true
    }

    /** macOS `resetRun()`: Try Again and Update Another Board. */
    fun resetRun() {
        confirmed = false
        rebootRequested = false
        installer.reset()
    }

    /** macOS `.onChange(of: installer.state)`. */
    fun observe(state: FirmwareInstallState) {
        when (state) {
            is FirmwareInstallState.Ready -> lastSeenChip = state.board.chip
            is FirmwareInstallState.WaitingForVolume -> lastSeenChip = state.chip
            is FirmwareInstallState.WaitingForPermission -> lastSeenChip = state.chip
            is FirmwareInstallState.NeedsConfirmation -> lastSeenChip = state.chip
            else -> Unit
        }
    }
}
