package se.bofr.dspi.app

import android.app.Application
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.AlinkSenderView
import se.bofr.dspi.bridge.CoreTools
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DeviceKind
import se.bofr.dspi.core.DeviceProtocol
import se.bofr.dspi.core.ExtEnvelope
import se.bofr.dspi.core.FirmwareGeneration
import se.bofr.dspi.core.FirmwareGuardDecision
import se.bofr.dspi.core.FirmwareGuardReason
import se.bofr.dspi.core.FirmwareGuardVerdict
import se.bofr.dspi.core.FirmwareLineage
import se.bofr.dspi.core.FirmwareTarget
import se.bofr.dspi.core.FirmwareUpdateRule
import se.bofr.dspi.core.FirmwareUpdateVerdict
import se.bofr.dspi.core.ProtocolGeneration
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.firmware.BoardAccess
import se.bofr.dspi.feature.firmware.BootloaderBoard
import se.bofr.dspi.feature.firmware.EntryCheck
import se.bofr.dspi.feature.firmware.FakeBootloaderLocator
import se.bofr.dspi.feature.firmware.FirmwareDeviceInfo
import se.bofr.dspi.feature.firmware.FirmwareGuarding
import se.bofr.dspi.feature.firmware.FirmwareImage
import se.bofr.dspi.feature.firmware.FirmwareInstaller
import se.bofr.dspi.feature.firmware.FirmwareUpdateModel
import se.bofr.dspi.feature.firmware.FirmwareUpdateView
import se.bofr.dspi.feature.firmware.FirmwareVerifying
import se.bofr.dspi.feature.firmware.FirmwareWriter
import se.bofr.dspi.feature.firmware.GENERATION_CHANGE_TAG
import se.bofr.dspi.feature.firmware.GuardClearance
import se.bofr.dspi.feature.firmware.PHRASE_FIELD_TAG
import se.bofr.dspi.feature.firmware.Uf2Image
import se.bofr.dspi.feature.settings.ALINK_SENDER_TAG
import se.bofr.dspi.feature.settings.DEVICE_INFO_TAG
import se.bofr.dspi.ui.DspiTheme

/**
 * WP PGAND on screen: the update confirmation asks the core's typed phrase in an empty field
 * (never filled in), shows the generation change and reboots the board only once the check
 * cleared; the alink sender gets its own page in place of Home and no DSP screen.
 */
@RunWith(DspiRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// Default window: with a w400dp-h1600dp window the dialog never idles under Robolectric (window sizing)
@Config(sdk = [35], application = Application::class)
class UpdateCheckAndSenderWpPgandTest {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    @After
    fun stop() {
        scope.cancel()
    }

    private val phrase = "I ACCEPT LOSING THE EXTENSIONS"
    private val change = "This update moves the device to firmware 1.1.7's paged protocol."

    private fun verdict(allowed: Boolean) = FirmwareUpdateVerdict(
        FirmwareGuardVerdict(
            allowed,
            if (allowed) FirmwareGuardDecision.CONFIRMED else FirmwareGuardDecision.NEEDS_CONFIRMATION,
            FirmwareGuardReason.IMAGE_UNKNOWN,
            "This board runs this fork's firmware, and this image may be upstream's.",
            if (allowed) "" else phrase,
            0,
            1200
        ),
        FirmwareUpdateRule.LINEAGE_LOSS,
        FirmwareLineage.UNKNOWN,
        FirmwareGeneration.PAGED,
        change
    )

    private object NoWriter : FirmwareWriter {
        override fun isReady(board: BootloaderBoard) = true

        override fun hasAccess(board: BootloaderBoard) = true

        override suspend fun obtainAccess(board: BootloaderBoard) = BoardAccess.GRANTED

        override suspend fun write(
            board: BootloaderBoard,
            image: FirmwareImage,
            uf2: Uf2Image,
            clearance: GuardClearance,
            progress: (Double) -> Unit
        ) = throw AssertionError("nothing is written here")
    }

    /** A state write reaches composition through an apply notification the paused clock does not run; send it. */
    private fun settle() {
        rule.runOnUiThread { androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(1_000)
    }

    private fun inDialog(text: String) = rule.onNode(hasText(text) and hasClickAction() and hasAnyAncestor(isDialog()))

    @Test
    fun wpPgandThePhraseIsAskedInAnEmptyFieldBeforeTheReboot() {
        val image = FirmwareImage(BootloaderBoard.Chip.RP2350, "DSPi-RP2350-v1.1.7.uf2", null, uf2(), isBundled = false)
        val asked = mutableListOf<String>()
        val guard = object : FirmwareGuarding {
            override suspend fun recordRunningBoard(): FirmwareTarget = CoreTools.targetInBootloader()

            override fun forgetBoard() = Unit

            override fun check(image: ByteArray, typed: String) = verdict(typed == phrase).also { asked += typed }

            override suspend fun checkWrite(image: ByteArray, typed: String) = check(image, typed)
        }
        val verifier = FirmwareVerifying { null }
        val installer = FirmwareInstaller(FakeBootloaderLocator(), verifier, NoWriter, { image }, guard, scope, Dispatchers.Main.immediate)
        val model = FirmwareUpdateModel(installer, bundled = { image }, picobootWriter = NoWriter)
        var reboots = 0
        // A paused clock (as AN31): the field's cursor blinks forever, so the rule never idles by itself
        rule.mainClock.autoAdvance = false
        rule.setContent {
            DspiTheme {
                FirmwareUpdateView(
                    model,
                    FirmwareDeviceInfo(true, null, chip = BootloaderBoard.Chip.RP2350, hasBootsel = true),
                    enterBootloader = { reboots++ },
                    onNavigate = {},
                    onBack = {}
                )
            }
        }
        rule.onNode(hasText("Update Firmware") and hasClickAction()).performClick()
        settle()
        // Lookups count first: a failed single-node lookup prints the whole tree (minutes under Robolectric)
        assertTrue("${model.installer.entryCheck.value}", model.installer.entryCheck.value is EntryCheck.Ask)
        assertEquals(1, rule.onAllNodes(hasTestTag(PHRASE_FIELD_TAG)).fetchSemanticsNodes().size)
        val field = rule.onNode(hasTestTag(PHRASE_FIELD_TAG))
        field.assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText, androidx.compose.ui.text.AnnotatedString("")))
        assertEquals(1, rule.onAllNodesWithTag(GENERATION_CHANGE_TAG, useUnmergedTree = true).fetchSemanticsNodes().size)
        inDialog("Continue").assertIsNotEnabled()
        assertEquals(
            0,
            rule.onAllNodes(hasText("Update Firmware") and hasClickAction() and hasAnyAncestor(isDialog())).fetchSemanticsNodes().size
        )
        field.performTextInput(phrase)
        settle()
        inDialog("Continue").assertIsEnabled().performClick()
        settle()
        assertEquals(0, reboots)
        inDialog("Update Firmware").assertIsEnabled().performClick()
        settle()
        assertEquals(1, reboots)
        assertEquals(listOf("", phrase), asked)
    }

    private val sender =
        DeviceProtocol(ProtocolGeneration.LEGACY, 0, 0, emptyList(), ExtEnvelope.LEGACY, 0, "DSPi", DeviceKind.ALINK_SENDER)

    @Test
    fun wpPgandTheAlinkSenderGetsItsPageAndNoDspScreen() {
        val real = DspiBridgeImpl(FakeDspiCore(), scope)
        val protocol = MutableStateFlow<DeviceProtocol?>(sender)
        val bridge = object : DspiBridge by real {
            override val deviceProtocol: StateFlow<DeviceProtocol?> get() = protocol

            override suspend fun fetchAlinkSender() = AlinkSenderView(error = "not on this test")
        }
        rule.mainClock.autoAdvance = false
        rule.setContent { AppScaffold(bridge, ConnectionStrip.Usb, onExitDemo = {}, onRequestPermission = {}) }
        settle()
        settle()
        assertEquals(1, rule.onAllNodesWithTag(ALINK_SENDER_TAG, useUnmergedTree = true).fetchSemanticsNodes().size)
        assertEquals(1, rule.onAllNodesWithTag(DEVICE_INFO_TAG, useUnmergedTree = true).fetchSemanticsNodes().size)
        assertEquals("no Home tools", 0, rule.onAllNodesWithContentDescription("Tools menu").fetchSemanticsNodes().size)
    }

    /** A one-block RP2350 UF2 (the installer parses it before the check). */
    private fun uf2(): ByteArray {
        val b = java.nio.ByteBuffer.allocate(512).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        b.putInt(0x0A324655)
        b.putInt(0x9E5D5157.toInt())
        b.putInt(0x00002000)
        b.putInt(0x10000000)
        b.putInt(256)
        b.putInt(0)
        b.putInt(1)
        b.putInt(0xE48BFF59.toInt())
        b.put(ByteArray(256))
        b.putInt(508, 0x0AB16F30)
        return b.array()
    }
}
