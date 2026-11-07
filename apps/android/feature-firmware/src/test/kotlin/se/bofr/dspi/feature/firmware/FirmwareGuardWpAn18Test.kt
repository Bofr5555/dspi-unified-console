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
import se.bofr.dspi.core.FirmwareGuardBoard
import se.bofr.dspi.core.FirmwareGuardBoardSource
import se.bofr.dspi.core.FirmwareGuardDecision
import se.bofr.dspi.core.FirmwareGuardReason
import se.bofr.dspi.core.FirmwareGuardVerdict

/**
 * WP AN18: CG8's firmware guard on the Kotlin flashing path. Every write, on
 * PICOBOOT and on the drive, goes through it: the installer cannot be built
 * without a guard, writes only after a verdict that allows the image, and a
 * writer refuses an image no verdict cleared. Real flashing stays a
 * coordinator step [HW].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class FirmwareGuardWpAn18Test {
    private val board = BootloaderBoard(BootloaderBoard.Chip.RP2350, "/dev/bus/usb/001/005")
    private val v = FirmwareVersion(1, 1, 7)
    private val image = firmwareImage(TestUf2.image(16), v)
    private val needsPhrase = guardVerdict(
        allowed = false,
        decision = FirmwareGuardDecision.NEEDS_CONFIRMATION,
        reason = FirmwareGuardReason.BOARD_UNKNOWN,
        message = "This board cannot say what core voltage it needs.",
        phrase = "I understand"
    )

    private fun TestScope.installer(guard: FirmwareGuarding, writer: FirmwareWriter, boards: List<BootloaderBoard> = listOf(board)) =
        FirmwareInstaller(FakeBootloaderLocator(boards), StubVerifier(v), writer, {
            image
        }, guard, this, StandardTestDispatcher(testScheduler))

    @Test fun wpAn18ARefusedImageIsNeverWritten() = runTest {
        val writer = FakeWriter(access = false)
        val guard =
            FakeGuard { guardVerdict(false, FirmwareGuardDecision.REFUSED, FirmwareGuardReason.IMAGE_INVALID, "This is not firmware.") }
        val i = installer(guard, writer)
        i.install(board)
        advanceUntilIdle()
        assertEquals(FirmwareInstallState.Failed(FirmwareInstallError.GuardBlocked("This is not firmware.")), i.state.value)
        assertTrue(writer.written.isEmpty())
        assertEquals("no permission dialog either", 0, writer.accessRequests)
        assertEquals(listOf(""), guard.checks)
    }

    @Test fun wpAn18ConfirmationGatesTheWriteUntilThePhraseIsTyped() = runTest {
        val writer = FakeWriter()
        val guard = FakeGuard { typed ->
            if (typed ==
                "I understand"
            ) {
                guardVerdict(decision = FirmwareGuardDecision.CONFIRMED)
            } else {
                needsPhrase
            }
        }
        val i = installer(guard, writer)
        i.beginWatching()
        i.installWhenReady()
        advanceUntilIdle()
        assertEquals(
            FirmwareInstallState.NeedsConfirmation(board.chip, needsPhrase.message, "I understand", wrongAnswer = false),
            i.state.value
        )
        assertTrue(writer.written.isEmpty())

        i.confirm("i understand")
        advanceUntilIdle()
        assertEquals(
            FirmwareInstallState.NeedsConfirmation(board.chip, needsPhrase.message, "I understand", wrongAnswer = true),
            i.state.value
        )
        assertTrue(writer.written.isEmpty())

        i.confirm("I understand")
        advanceUntilIdle()
        assertEquals(FirmwareInstallState.Verified(v), i.state.value)
        assertEquals(listOf(board to image), writer.written)
        assertEquals(listOf("", "i understand", "I understand"), guard.checks)
    }

    @Test fun wpAn18ConfirmOutsideTheQuestionDoesNothing() = runTest {
        val writer = FakeWriter()
        val guard = FakeGuard()
        val i = installer(guard, writer)
        i.confirm("I understand")
        advanceUntilIdle()
        assertTrue(guard.checks.isEmpty())
        assertTrue(writer.written.isEmpty())
    }

    @Test fun wpAn18AGuardThatFailsBlocks() = runTest {
        val writer = FakeWriter()
        val i = installer(FakeGuard { throw IllegalStateException("core gone") }, writer)
        i.install(board)
        advanceUntilIdle()
        assertTrue((i.state.value as FirmwareInstallState.Failed).error is FirmwareInstallError.GuardBlocked)
        assertTrue(writer.written.isEmpty())
    }

    @Test fun wpAn18PicobootRefusesAnImageNoVerdictCleared() = runTest {
        var opens = 0
        val writer = PicobootFirmwareWriter(AlwaysAllowed) {
            opens++
            FakePicobootDevice()
        }
        val other = firmwareImage(image.bytes.copyOf(), v)
        val e = runCatching { writer.write(board, image, Uf2Image.parse(image.bytes), clearance(other)) {} }.exceptionOrNull()
        assertTrue(e is FirmwareInstallError.GuardBlocked)
        assertEquals("the board is never opened", 0, opens)
        val refused = GuardClearance(image.bytes, needsPhrase)
        assertTrue(
            runCatching {
                writer.write(board, image, Uf2Image.parse(image.bytes), refused) {}
            }.exceptionOrNull() is FirmwareInstallError.GuardBlocked
        )
        assertEquals(0, opens)
    }

    @Test fun wpAn18TheDriveRefusesAnImageNoVerdictCleared() = runTest {
        var creates = 0
        val volume = object : Uf2Volume {
            override val name = "RP2350"

            override fun isMounted() = true

            override fun create(fileName: String): Uf2Sink {
                creates++
                throw AssertionError("nothing may be written")
            }
        }
        val other = firmwareImage(image.bytes.copyOf(), v)
        val e = runCatching {
            DriveFirmwareWriter(volume).write(board, image, Uf2Image.parse(image.bytes), clearance(other)) {}
        }.exceptionOrNull()
        assertTrue(e is FirmwareInstallError.GuardBlocked)
        assertEquals(0, creates)
    }

    @Test fun wpAn18TheRunningBoardIsAskedBeforeItReboots() = runTest {
        val guard = FakeGuard()
        val m =
            FirmwareUpdateModel(installer(guard, FakeWriter(), boards = emptyList()), bundled = { image }, picobootWriter = FakeWriter())
        m.installer.beginWatching()
        var readsAtReboot = -1
        m.start(deviceConnected = true) { readsAtReboot = guard.boardReads }
        advanceUntilIdle()
        assertEquals(1, readsAtReboot)
        m.resetRun()
        assertEquals("the next board is one nobody asked", 1, guard.forgotten)
    }

    @Test fun wpAn18ABoardAlreadyInBootselIsNotAsked() = runTest {
        val guard = FakeGuard()
        val m = FirmwareUpdateModel(installer(guard, FakeWriter()), bundled = { image }, picobootWriter = FakeWriter())
        m.installer.beginWatching()
        m.start(deviceConnected = false) { throw AssertionError("no reboot") }
        advanceUntilIdle()
        assertEquals(0, guard.boardReads)
        assertEquals(listOf(""), guard.checks)
    }

    @Test fun wpAn18CoreGuardUsesTheRecordItTook() = runTest {
        val running = FirmwareGuardBoard(1200, FirmwareGuardBoardSource.EXT_CORE_VREG)
        val tools = RecordingTools(running)
        val g = CoreFirmwareGuard(tools)
        g.check(byteArrayOf(1), "")
        assertEquals(CoreTools.UNKNOWN_BOARD, tools.boards.last())
        g.recordRunningBoard()
        g.check(byteArrayOf(1), "x")
        assertEquals(running, tools.boards.last())
        g.forgetBoard()
        g.check(byteArrayOf(1), "")
        assertEquals(CoreTools.UNKNOWN_BOARD, tools.boards.last())
    }

    @Test fun wpAn18UnavailableToolsBlock() {
        val verdict = CoreTools.Unavailable.firmwareGuardCheck(byteArrayOf(1, 2), CoreTools.UNKNOWN_BOARD, "")
        assertFalse(verdict.allowed)
        assertEquals(FirmwareGuardDecision.REFUSED, verdict.decision)
    }

    private object AlwaysAllowed : BoardPermissions {
        override fun hasPermission(board: BootloaderBoard) = true

        override suspend fun requestPermission(board: BootloaderBoard) = BoardAccess.GRANTED

        override fun isAttached(board: BootloaderBoard) = true
    }

    private class RecordingTools(private val running: FirmwareGuardBoard) : CoreTools {
        val boards = mutableListOf<FirmwareGuardBoard>()

        override suspend fun firmwareGuardBoard() = running

        override fun firmwareGuardBoardInBootloader() = CoreTools.UNKNOWN_BOARD

        override fun firmwareGuardCheck(image: ByteArray, board: FirmwareGuardBoard, typedConfirmation: String): FirmwareGuardVerdict {
            boards += board
            return guardVerdict()
        }

        override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch): AutoEqDatabaseInfo =
            throw UnsupportedOperationException()
    }
}
