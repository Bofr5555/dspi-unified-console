package se.bofr.dspi.feature.firmware

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.CoreTools
import se.bofr.dspi.core.AutoEqDatabaseInfo
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.FirmwareGeneration
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.FirmwareGuardDecision
import se.bofr.dspi.core.FirmwareGuardReason
import se.bofr.dspi.core.FirmwareGuardVerdict
import se.bofr.dspi.core.FirmwareLineage
import se.bofr.dspi.core.FirmwareTarget
import se.bofr.dspi.core.FirmwareUpdateVerdict

/**
 * Batch review PG-D part 2: Android's write is Kotlin's, so its last check asks the
 * core's write gate (`firmware_write_check`), which judges by the target the core
 * itself read and still keeps. A target kept in Kotlin is never the judge at the
 * write: it is handed over as a voltage record that can only add doubt, and it is
 * dropped when the update was not started.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class FirmwareWriteGateReviewPgd2Test {
    private val board = BootloaderBoard(BootloaderBoard.Chip.RP2350, "/dev/bus/usb/001/005")
    private val v = FirmwareVersion(1, 1, 7)
    private val image = firmwareImage(TestUf2.image(16), v)
    private val voltagePhrase = "I ACCEPT THE CORE VOLTAGE RISK"

    private val stockPico = FirmwareTarget(
        FirmwareGuardBoard(1150, FirmwareGuardBoardSource.STATUS_BLOCK),
        boardId = 0,
        lineage = FirmwareLineage.UPSTREAM,
        generation = FirmwareGeneration.LEGACY
    )

    /** The core as the installer sees it: the entry check is clear; the gate answers [gate]. */
    private class GateTools(private val target: FirmwareTarget, var gate: (typed: String) -> FirmwareUpdateVerdict) : CoreTools {
        val judgedBefore = mutableListOf<FirmwareTarget>()
        val gateBoards = mutableListOf<FirmwareGuardBoard>()
        val gateTyped = mutableListOf<String>()

        override suspend fun firmwareGuardBoard() = target.voltage

        override fun firmwareGuardBoardInBootloader() = CoreTools.UNKNOWN_BOARD

        override fun firmwareGuardCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareGuardVerdict =
            throw AssertionError("the voltage rule alone is not asked")

        override suspend fun firmwareTarget() = target

        override fun firmwareUpdateCheck(image: ByteArray, target: FirmwareTarget, typedConfirmation: String): FirmwareUpdateVerdict {
            judgedBefore += target
            return updateVerdict()
        }

        override suspend fun firmwareWriteCheck(
            image: ByteArray,
            board: FirmwareGuardBoard,
            typedConfirmation: String
        ): FirmwareUpdateVerdict {
            gateBoards += board
            gateTyped += typedConfirmation
            return gate(typedConfirmation)
        }

        override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo =
            throw UnsupportedOperationException()
    }

    private fun asksTheVoltagePhrase(typed: String) = if (typed == voltagePhrase) {
        updateVerdict(guardVerdict(decision = FirmwareGuardDecision.CONFIRMED))
    } else {
        updateVerdict(
            guardVerdict(
                false,
                FirmwareGuardDecision.NEEDS_CONFIRMATION,
                FirmwareGuardReason.IMAGE_UNKNOWN,
                "This board needs 1.20 V.",
                voltagePhrase
            )
        )
    }

    private fun TestScope.installer(guard: FirmwareGuarding, writer: FakeWriter, locator: FakeBootloaderLocator) =
        FirmwareInstaller(locator, StubVerifier(v), writer, { image }, guard, this, StandardTestDispatcher(testScheduler))

    private fun TestScope.model(tools: CoreTools, writer: FakeWriter, locator: FakeBootloaderLocator) = FirmwareUpdateModel(
        installer(CoreFirmwareGuard(tools), writer, locator),
        bundled = { image },
        picobootWriter = writer
    )

    @Test fun reviewPgd2TheWriteAsksTheCoresGateNotTheTargetKeptHere() = runTest {
        // The entry check on the board read here is clear; the core, which meanwhile opened
        // and rebooted another board (a K340), is not: its gate asks the voltage phrase.
        val tools = GateTools(stockPico, ::asksTheVoltagePhrase)
        val writer = FakeWriter()
        val locator = FakeBootloaderLocator()
        val m = model(tools, writer, locator)
        m.installer.beginWatching()
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        advanceUntilIdle()
        assertEquals(EntryCheck.Clear(""), m.installer.entryCheck.value)
        assertTrue(m.start(deviceConnected = true) {})
        locator.boards = listOf(board)
        locator.emit()
        advanceUntilIdle()
        assertTrue("nothing is written on the clear entry check alone", writer.written.isEmpty())
        val question = m.installer.state.value as FirmwareInstallState.NeedsConfirmation
        assertEquals(voltagePhrase, question.phrase)
        assertEquals("the entry check ran once, the gate decided the write", 1, tools.judgedBefore.size)
        // The record kept here goes along only as a voltage record
        assertEquals(listOf(stockPico.voltage), tools.gateBoards)
        m.installer.confirm("I ACCEPT LOSING THE EXTENSIONS")
        advanceUntilIdle()
        assertTrue(writer.written.isEmpty())
        m.installer.confirm(voltagePhrase)
        advanceUntilIdle()
        assertEquals(listOf(board to image), writer.written)
        assertEquals(listOf("", "I ACCEPT LOSING THE EXTENSIONS", voltagePhrase), tools.gateTyped)
    }

    @Test fun reviewPgd2ADismissedCheckKeepsNoBoardForALaterOne() = runTest {
        val tools = GateTools(stockPico) { updateVerdict() }
        val writer = FakeWriter()
        val locator = FakeBootloaderLocator()
        val m = model(tools, writer, locator)
        m.installer.beginWatching()
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        advanceUntilIdle()
        assertEquals(EntryCheck.Clear(""), m.installer.entryCheck.value)
        // Not started: nothing was sent, the dialog is closed
        m.dismissUpdate()
        // Later a board is put in BOOTSEL by hand and written: it is one nobody asked
        m.start(deviceConnected = false) { throw AssertionError("no reboot") }
        locator.boards = listOf(board)
        locator.emit()
        advanceUntilIdle()
        assertEquals(listOf(CoreTools.UNKNOWN_BOARD), tools.gateBoards)
        assertFalse(tools.gateBoards.contains(stockPico.voltage))
    }

    @Test fun reviewPgd2TheBoardThisRunRebootedKeepsItsRecordForTheGate() = runTest {
        val tools = GateTools(stockPico) { updateVerdict() }
        val writer = FakeWriter()
        val locator = FakeBootloaderLocator()
        val m = model(tools, writer, locator)
        m.installer.beginWatching()
        m.requestUpdate(deviceConnected = true, chip = board.chip)
        advanceUntilIdle()
        assertTrue(m.start(deviceConnected = true) {})
        // Closing the dialog's state after the commitment drops nothing
        m.dismissUpdate()
        locator.boards = listOf(board)
        locator.emit()
        advanceUntilIdle()
        assertEquals(listOf(stockPico.voltage), tools.gateBoards)
        assertEquals(listOf(board to image), writer.written)
    }

    @Test fun reviewPgd2TheBundledDspImageIsNotOfferedToTheAlinkSender() = runTest {
        val guard = FakeGuard()
        val unused = FakeWriter()
        val m = FirmwareUpdateModel(installer(guard, unused, FakeBootloaderLocator()), bundled = { image }, picobootWriter = unused)
        m.requestUpdate(deviceConnected = true, chip = board.chip, alinkSender = true)
        advanceUntilIdle()
        assertEquals(EntryCheck.Refused(SENDER_BUNDLED_TEXT, noBootsel = false), m.installer.entryCheck.value)
        assertEquals("nothing is read or judged", 0, guard.boardReads)
        assertTrue(guard.checks.isEmpty())
        assertFalse(m.start(deviceConnected = true) { throw AssertionError("no reboot") })
        // Its own image, chosen by the person, is judged as any other
        m.dismissUpdate()
        m.chooseLocal(firmwareImage(TestUf2.image(16), version = null))
        m.requestUpdate(deviceConnected = true, chip = board.chip, alinkSender = true)
        advanceUntilIdle()
        assertEquals(EntryCheck.Clear(""), m.installer.entryCheck.value)
        assertEquals(1, guard.boardReads)
    }

    @Test fun reviewPgd2WithoutACoreTheGateBlocks() = runTest {
        val v = CoreTools.Unavailable.firmwareWriteCheck(byteArrayOf(1), CoreTools.UNKNOWN_BOARD, voltagePhrase)
        assertFalse(v.verdict.allowed)
        assertEquals(FirmwareGuardDecision.REFUSED, v.verdict.decision)
    }
}
