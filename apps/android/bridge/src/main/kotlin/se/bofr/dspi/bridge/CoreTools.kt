// WP AN18: the core calls a feature needs beside the device's state: CG8's
// firmware guard, for the Kotlin flashing path (PICOBOOT and the drive), and
// CG6's AutoEQ update with a host-supplied fetch. The guard's rules and the
// update's checks stay in the core; this only carries the calls.
package se.bofr.dspi.bridge

import se.bofr.dspi.core.AutoEqDatabaseInfo
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.FirmwareGuardDecision
import se.bofr.dspi.core.FirmwareGuardReason
import se.bofr.dspi.core.FirmwareGuardVerdict

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

    companion object {
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

    override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo = bridge.core.autoeqUpdate(dbPath, url, fetch)

    private fun ByteArray.toWire(): List<Int> = List(size) { this[it].toInt() and 0xFF }
}
