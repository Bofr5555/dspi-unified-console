package se.bofr.dspi.feature.firmware

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.CoreTools
import se.bofr.dspi.core.AutoEqDatabaseInfo
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.DeviceKind
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

/**
 * WP PGAND: the Android update screen judges with the core's update check
 * (`firmware_target` + `firmware_update_check`) BEFORE the bootloader entry:
 * the typed phrase is asked while the board still runs (never filled in), the
 * generation change is shown, a board without BOOTSEL is offered no image and
 * is not rebooted, and the write is judged on the target read from that board
 * with the answer given to it. Real flashing stays a coordinator step [HW].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class FirmwareUpdateCheckWpPgandTest {
    private val board = BootloaderBoard(BootloaderBoard.Chip.RP2350, "/dev/bus/usb/001/005")
    private val v = FirmwareVersion(1, 1, 7)
    private val image = firmwareImage(TestUf2.image(16), v)
    private val lineage = "I ACCEPT LOSING THE EXTENSIONS"
    private val change = "This update moves the device to firmware 1.1.7's paged protocol."
    private val lossMessage = "This board runs this fork's firmware, and this image may be upstream's."

    /** The lineage rule as the core words it: the phrase, or clear once typed. */
    private fun lineageGuard() = FakeGuard().apply {
        update = { typed ->
            if (typed == lineage) {
                updateVerdict(guardVerdict(decision = FirmwareGuardDecision.CONFIRMED), FirmwareUpdateRule.LINEAGE_LOSS, change)
            } else {
                updateVerdict(
                    guardVerdict(false, FirmwareGuardDecision.NEEDS_CONFIRMATION, FirmwareGuardReason.IMAGE_UNKNOWN, lossMessage, lineage),
                    FirmwareUpdateRule.LINEAGE_LOSS,
                    change
                )
            }
        }
    }

    private fun TestScope.model(guard: FirmwareGuarding, writer: FakeWriter, locator: FakeBootloaderLocator = FakeBootloaderLocator()) =
        FirmwareUpdateModel(
            FirmwareInstaller(locator, StubVerifier(v), writer, { image }, guard, this, StandardTestDispatcher(testScheduler)),
            bundled = { image },
            picobootWriter = writer
        )

    @Test fun wpPgandTheCheckRunsBeforeTheEntryAndAClearImageReboots() = runTest {
        val guard = FakeGuard().apply { generationChange = change }
        val m = model(guard, FakeWriter())
        m.installer.beginWatching()
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        assertTrue(m.confirmingUpdate)
        assertEquals(EntryCheck.Checking, m.installer.entryCheck.value)
        advanceUntilIdle()
        assertEquals("the target is read before anything", 1, guard.boardReads)
        assertEquals(EntryCheck.Clear(change), m.installer.entryCheck.value)
        assertEquals(listOf(""), guard.checks)
        var reboots = 0
        assertTrue(m.start(deviceConnected = true) { reboots++ })
        assertEquals(1, reboots)
        assertTrue(m.confirmed)
    }

    @Test fun wpPgandNoRebootUntilTheCheckCleared() = runTest {
        val m = model(lineageGuard(), FakeWriter())
        m.installer.beginWatching()
        var reboots = 0
        // Not yet asked: nothing goes out
        assertFalse(m.start(deviceConnected = true) { reboots++ })
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        assertFalse("still checking", m.start(deviceConnected = true) { reboots++ })
        advanceUntilIdle()
        assertFalse("the phrase is not typed", m.start(deviceConnected = true) { reboots++ })
        assertEquals(0, reboots)
        assertFalse(m.confirmed)
        assertFalse(m.installer.isArmed)
    }

    @Test fun wpPgandTheLineagePhraseIsAskedBeforeTheEntryAndCarriedToTheWrite() = runTest {
        val guard = lineageGuard()
        val writer = FakeWriter()
        val locator = FakeBootloaderLocator()
        val m = model(guard, writer, locator)
        m.installer.beginWatching()
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        advanceUntilIdle()
        // The question carries the phrase to show and nothing typed: the field starts empty
        assertEquals(EntryCheck.Ask(lossMessage, lineage, change, wrongAnswer = false), m.installer.entryCheck.value)
        m.installer.answerEntry("i accept losing the extensions")
        advanceUntilIdle()
        assertEquals(EntryCheck.Ask(lossMessage, lineage, change, wrongAnswer = true), m.installer.entryCheck.value)
        var reboots = 0
        assertFalse(m.start(deviceConnected = true) { reboots++ })
        m.installer.answerEntry(lineage)
        advanceUntilIdle()
        assertEquals(EntryCheck.Clear(change), m.installer.entryCheck.value)
        assertTrue(m.start(deviceConnected = true) { reboots++ })
        assertEquals(1, reboots)
        assertTrue(writer.written.isEmpty())
        // The board arrives in BOOTSEL: the write is judged with the same answer, and goes out
        locator.boards = listOf(board)
        locator.emit()
        advanceUntilIdle()
        assertEquals(FirmwareInstallState.Verified(v), m.installer.state.value)
        assertEquals(listOf(board to image), writer.written)
        assertEquals(listOf("", "i accept losing the extensions", lineage, lineage), guard.checks)
        assertEquals("the write's verdict keeps the generation change", change, m.installer.lastVerdict?.generationChange)
    }

    @Test fun wpPgandABoardWithoutBootselIsOfferedNoImageAndNotRebooted() = runTest {
        val noBootsel = "No image is offered for this device (DSPi Zero): it has no BOOTSEL bootloader this app can update it through."
        val guard = FakeGuard().apply {
            update =
                {
                    updateVerdict(
                        guardVerdict(false, FirmwareGuardDecision.REFUSED, FirmwareGuardReason.BOARD_UNKNOWN, noBootsel),
                        FirmwareUpdateRule.NO_BOOTSEL
                    )
                }
        }
        val writer = FakeWriter()
        val m = model(guard, writer)
        m.installer.beginWatching()
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        advanceUntilIdle()
        assertEquals(EntryCheck.Refused(noBootsel, noBootsel = true), m.installer.entryCheck.value)
        // Whatever is typed: no question is open, nothing is re-judged
        m.installer.answerEntry("I ACCEPT LOSING THE EXTENSIONS")
        advanceUntilIdle()
        assertEquals(listOf(""), guard.checks)
        var reboots = 0
        assertFalse(m.start(deviceConnected = true) { reboots++ })
        assertEquals(0, reboots)
        assertFalse(m.confirmed)
        assertTrue(writer.written.isEmpty())
        m.dismissUpdate()
        assertNull(m.installer.entryCheck.value)
        assertFalse(m.confirmingUpdate)
    }

    @Test fun wpPgandAWriteJudgedNoBootselIsRefusedWhateverIsTyped() = runTest {
        val guard = FakeGuard().apply {
            update =
                {
                    updateVerdict(
                        guardVerdict(false, FirmwareGuardDecision.REFUSED, FirmwareGuardReason.BOARD_UNKNOWN, "No image."),
                        FirmwareUpdateRule.NO_BOOTSEL
                    )
                }
        }
        val writer = FakeWriter()
        val m = model(guard, writer, FakeBootloaderLocator(listOf(board)))
        m.installer.beginWatching()
        m.start(deviceConnected = false) { throw AssertionError("no reboot") }
        advanceUntilIdle()
        assertEquals(FirmwareInstallState.Failed(FirmwareInstallError.GuardBlocked("No image.")), m.installer.state.value)
        assertTrue(writer.written.isEmpty())
    }

    @Test fun wpPgandABoardAlreadyInBootselIsAskedAtTheWriteWithTheGenerationChange() = runTest {
        val guard = lineageGuard()
        val writer = FakeWriter()
        val m = model(guard, writer, FakeBootloaderLocator(listOf(board)))
        m.installer.beginWatching()
        m.start(deviceConnected = false) { throw AssertionError("no reboot") }
        advanceUntilIdle()
        assertEquals(0, guard.boardReads)
        assertEquals(
            FirmwareInstallState.NeedsConfirmation(board.chip, lossMessage, lineage, wrongAnswer = false, generationChange = change),
            m.installer.state.value
        )
        m.installer.confirm(lineage)
        advanceUntilIdle()
        assertEquals(listOf(board to image), writer.written)
    }

    @Test fun wpPgandAnImageForTheOtherChipIsRefusedBeforeTheEntry() = runTest {
        val guard = FakeGuard()
        val m = model(guard, FakeWriter())
        m.requestUpdate(deviceConnected = true, chip = BootloaderBoard.Chip.RP2040)
        advanceUntilIdle()
        val refused = m.installer.entryCheck.value as EntryCheck.Refused
        assertFalse(refused.noBootsel)
        assertTrue(refused.message, refused.message.contains("RP2040"))
        assertTrue("not judged further", guard.checks.isEmpty())
        assertFalse(m.start(deviceConnected = true) { throw AssertionError("no reboot") })
    }

    @Test fun wpPgandTheWriteIsJudgedOnTheTargetReadFromThisBoard() = runTest {
        val read = FirmwareTarget(
            FirmwareGuardBoard(1200, FirmwareGuardBoardSource.EXT_CORE_VREG),
            boardId = 0,
            lineage = FirmwareLineage.BOFR,
            generation = FirmwareGeneration.LEGACY,
            deviceKind = DeviceKind.DSP // -- WP IMGKIND --
        )
        val tools = TargetTools(read)
        val g = CoreFirmwareGuard(tools)
        assertSame(read, g.recordRunningBoard())
        g.check(byteArrayOf(1), "")
        g.check(byteArrayOf(1), lineage)
        assertEquals(listOf(read, read), tools.judged)
        assertEquals(listOf("", lineage), tools.typed)
        // Forgotten: the next board is one in BOOTSEL nobody asked
        g.forgetBoard()
        g.check(byteArrayOf(1), "")
        assertEquals(CoreTools.targetInBootloader(), tools.judged.last())
    }

    @Test fun wpPgandWithoutACoreNothingIsOffered() = runTest {
        val tools = CoreTools.Unavailable
        assertEquals(CoreTools.NO_TARGET, tools.firmwareTarget())
        assertNull(CoreTools.NO_TARGET.boardId)
        val v = tools.firmwareUpdateCheck(byteArrayOf(1), CoreTools.NO_TARGET, lineage)
        assertFalse(v.verdict.allowed)
        assertEquals(FirmwareGuardDecision.REFUSED, v.verdict.decision)
    }

    private class TargetTools(private val target: FirmwareTarget) : CoreTools {
        val judged = mutableListOf<FirmwareTarget>()
        val typed = mutableListOf<String>()

        override suspend fun firmwareGuardBoard() = target.voltage

        override fun firmwareGuardBoardInBootloader() = CoreTools.UNKNOWN_BOARD

        override fun firmwareGuardCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareGuardVerdict =
            throw AssertionError("the voltage rule alone is not asked")

        override suspend fun firmwareTarget() = target

        override fun firmwareUpdateCheck(image: ByteArray, target: FirmwareTarget, typedConfirmation: String): FirmwareUpdateVerdict {
            judged += target
            typed += typedConfirmation
            return updateVerdict()
        }

        override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo =
            throw UnsupportedOperationException()
    }
}
