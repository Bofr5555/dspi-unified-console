// WP AN18: the core calls a feature needs beside the device's state: CG8's
// firmware guard, for the Kotlin flashing path (PICOBOOT and the drive), and
// CG6's AutoEQ update with a host-supplied fetch. The guard's rules and the
// update's checks stay in the core; this only carries the calls.
package se.bofr.dspi.bridge

import se.bofr.dspi.core.AutoEqDatabaseInfo
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.BundledFirmware
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FirmwareGeneration
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.FirmwareGuardDecision
import se.bofr.dspi.core.FirmwareGuardReason
import se.bofr.dspi.core.FirmwareGuardVerdict
import se.bofr.dspi.core.FirmwareLineage
import se.bofr.dspi.core.FirmwareTarget
import se.bofr.dspi.core.FirmwareUpdateRule
import se.bofr.dspi.core.FirmwareUpdateVerdict

/** [DspiBridge.coreTools]. */
interface CoreTools {
    /**
     * `firmware_guard_board`: what the open board needs, read from it now on
     * the core's thread. Call before `enterBootloader`. Never fails: no device,
     * or an answer that cannot be believed, gives an unknown board.
     */
    suspend fun firmwareGuardBoard(): FirmwareGuardBoard

    /** `firmware_guard_board_in_bootloader`: a board nobody can ask, so the check asks the user. */
    fun firmwareGuardBoardInBootloader(): FirmwareGuardBoard

    /**
     * `firmware_guard_check`. Parses the whole image: call off the main
     * thread. Never throws; a failed call is a verdict that blocks.
     */
    fun firmwareGuardCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareGuardVerdict

    /**
     * `autoeq_update`: download a database from [url] through [fetch] and put
     * it at [dbPath] once the core knows it is one. Blocks for the request:
     * call off the main thread. Throws [DspiException].
     */
    fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo

    /**
     * -- WP SECFIX -- The label of a firmware file the user picked (SECFWV): null when the
     * core's manifest finds it is a bundled image byte for byte (`verify_bundled` with its
     * file [name]), else "Not a verified DSPi Console image" and what that means, the core's
     * notice row `firmware.not_verified`. Information, not a refusal. Hashes the image: call
     * off the main thread. Without a core nothing can be verified, so the label shows.
     */
    fun firmwareLabel(image: ByteArray, name: String): FirmwareLabel? = FirmwareLabel.FALLBACK

    // -- WP PGAND -- the update check (PLAN-PG section 1.8): the target, then the verdict on it
    /**
     * `firmware_target`: the open board as an update target, read now on the core's
     * thread (up to five control transfers; the core keeps it too). Call before
     * `enterBootloader` and keep it for [firmwareUpdateCheck]. Never fails: no device,
     * or a failed call, gives a target with no board, which is offered no image.
     */
    suspend fun firmwareTarget(): FirmwareTarget = NO_TARGET

    /**
     * `firmware_update_check`: the voltage rule on [target]'s record, no image for a board
     * without BOOTSEL, the lineage phrase, the generation change. Parses the whole image:
     * call off the main thread. Never throws; a failed call is a verdict that blocks.
     */
    fun firmwareUpdateCheck(image: ByteArray, target: FirmwareTarget, typedConfirmation: String): FirmwareUpdateVerdict =
        blockedUpdate("The firmware update check is not available.")

    // -- review PG-D2 --
    /**
     * `firmware_write_check`: the core's write gate, asked without writing. The last
     * check before a Kotlin write (PICOBOOT or the drive): the core judges by the target
     * it read itself from the board ([firmwareTarget], or its bootloader entry) and drops
     * when another device is opened, so a target kept here from another board cannot
     * clear an image. [board] only adds doubt: pass the record of the board this run
     * sent to its bootloader, else [firmwareGuardBoardInBootloader]. On the core's
     * thread. Never throws; a failed call is a verdict that blocks.
     */
    suspend fun firmwareWriteCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareUpdateVerdict =
        blockedUpdate("The firmware write check is not available.")

    companion object {
        // -- WP PGAND --
        /** No board known (no device, or the read failed): offered no image. */
        val NO_TARGET = FirmwareTarget(
            voltage = FirmwareGuardBoard(requiredMv = 0, source = FirmwareGuardBoardSource.NO_DEVICE),
            boardId = null,
            lineage = FirmwareLineage.UNKNOWN,
            generation = FirmwareGeneration.UNKNOWN,
            deviceKind = null // -- WP IMGKIND -- not known
        )

        /**
         * `FirmwareTarget::in_bootloader` (not exported): a board already in BOOTSEL has
         * BOOTSEL by being there (board 0); nothing else is known, so the voltage rule asks.
         */
        fun targetInBootloader(board: FirmwareGuardBoard = UNKNOWN_BOARD) = FirmwareTarget(
            voltage = board,
            boardId = 0,
            lineage = FirmwareLineage.UNKNOWN,
            generation = FirmwareGeneration.UNKNOWN,
            deviceKind = null // -- WP IMGKIND -- not known
        )

        /** The update verdict when the check could not be asked: nothing may be written. */
        fun blockedUpdate(message: String) = FirmwareUpdateVerdict(
            verdict = blocked(message),
            rule = FirmwareUpdateRule.NONE,
            imageLineage = FirmwareLineage.UNKNOWN,
            imageGeneration = FirmwareGeneration.UNKNOWN,
            generationChange = ""
        )

        /** An unknown board: what the guard is given when nothing better is known. */
        val UNKNOWN_BOARD = FirmwareGuardBoard(requiredMv = 0, source = FirmwareGuardBoardSource.BOOTLOADER)

        /** The verdict when the guard could not be asked: nothing may be written. */
        fun blocked(message: String) = FirmwareGuardVerdict(
            allowed = false,
            decision = FirmwareGuardDecision.REFUSED,
            reason = FirmwareGuardReason.IMAGE_INVALID,
            message = message,
            confirmationPhrase = "",
            imageMv = 0,
            requiredMv = 0
        )
    }

    /** For bridges without a core (test doubles): the guard blocks, the update is unsupported. */
    object Unavailable : CoreTools {
        override suspend fun firmwareGuardBoard() = UNKNOWN_BOARD

        override fun firmwareGuardBoardInBootloader() = UNKNOWN_BOARD

        override fun firmwareGuardCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String) =
            blocked("The firmware guard is not available.")

        override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo = throw DspiException.Unsupported()
    }
}

// -- WP SECFIX --
/** [CoreTools.firmwareLabel]: the label and its explanation, in the core's words. */
data class FirmwareLabel(val title: String, val text: String) {
    companion object {
        /** The notice row's key. */
        const val NOTICE_KEY = "firmware.not_verified"

        /** When the core cannot be asked: the row's title, no explanation. */
        val FALLBACK = FirmwareLabel("Not a verified DSPi Console image", "")
    }
}

internal class CoreToolsImpl(private val bridge: DspiBridgeImpl) : CoreTools {
    override suspend fun firmwareGuardBoard(): FirmwareGuardBoard =
        bridge.onFfi { runCatching { bridge.core.firmwareGuardBoard() }.getOrDefault(CoreTools.UNKNOWN_BOARD) }

    override fun firmwareGuardBoardInBootloader(): FirmwareGuardBoard =
        runCatching { bridge.core.firmwareGuardBoardInBootloader() }.getOrDefault(CoreTools.UNKNOWN_BOARD)

    override fun firmwareGuardCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareGuardVerdict = try {
        bridge.core.firmwareGuardCheck(image.toWire(), board, typedConfirmation)
    } catch (e: Exception) {
        // A failure of the call is no permission
        CoreTools.blocked("The firmware guard could not check this image: ${e.message ?: e.javaClass.simpleName}.")
    }

    override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo =
        bridge.core.autoeqUpdate(dbPath, url, fetch)

    // -- WP SECFIX --
    // A failed call verifies nothing: the label shows
    override fun firmwareLabel(image: ByteArray, name: String): FirmwareLabel? = runCatching {
        if (bridge.core.verifyBundled(image.toWire(), name) == BundledFirmware.VERIFIED) {
            null
        } else {
            val n = bridge.core.noticeForKey(FirmwareLabel.NOTICE_KEY, "")
            if (n.key == FirmwareLabel.NOTICE_KEY) FirmwareLabel(n.title, "${n.reason} ${n.action}") else FirmwareLabel.FALLBACK
        }
    }.getOrDefault(FirmwareLabel.FALLBACK)

    // -- WP PGAND --
    override suspend fun firmwareTarget(): FirmwareTarget =
        bridge.onFfi { runCatching { bridge.core.firmwareTarget() }.getOrDefault(CoreTools.NO_TARGET) }

    // A failure of the call is no permission
    override fun firmwareUpdateCheck(image: ByteArray, target: FirmwareTarget, typedConfirmation: String): FirmwareUpdateVerdict = try {
        bridge.core.firmwareUpdateCheck(image.toWire(), target, typedConfirmation)
    } catch (e: Exception) {
        CoreTools.blockedUpdate("The firmware update check could not judge this image: ${e.message ?: e.javaClass.simpleName}.")
    }

    // -- review PG-D2 -- a failure of the call is no permission
    override suspend fun firmwareWriteCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareUpdateVerdict =
        bridge.onFfi {
            runCatching { bridge.core.firmwareWriteCheck(image.toWire(), board, typedConfirmation) }.getOrElse {
                CoreTools.blockedUpdate("The firmware write check could not judge this image: ${it.message ?: it.javaClass.simpleName}.")
            }
        }

    private fun ByteArray.toWire(): List<Int> = List(size) { this[it].toInt() and 0xFF }
}
