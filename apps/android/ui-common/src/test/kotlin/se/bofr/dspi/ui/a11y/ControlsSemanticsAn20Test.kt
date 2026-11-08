package se.bofr.dspi.ui.a11y

import android.app.Application
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.ui.BypassCheckbox
import se.bofr.dspi.ui.CompactGainField
import se.bofr.dspi.ui.CpuMeter
import se.bofr.dspi.ui.CustomSlider
import se.bofr.dspi.ui.DspiIcons
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.HorizontalMeterBar
import se.bofr.dspi.ui.MeterReadout
import se.bofr.dspi.ui.ParameterRow
import se.bofr.dspi.ui.SegmentTab
import se.bofr.dspi.ui.SegmentedControl
import se.bofr.dspi.ui.SidebarIconButton

/**
 * WP AN20: the shared controls as TalkBack reads them, and ATF's checks over
 * them. Native graphics: Compose finds the nodes TalkBack may reach with
 * `android.graphics.Region`, which the legacy mode only stubs (every node
 * would be "covered" and left out).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ControlsSemanticsAn20Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private lateinit var view: View

    private fun show(content: @androidx.compose.runtime.Composable () -> Unit) = rule.setContent {
        view = LocalView.current
        DspiTheme(darkTheme = true) { content() }
    }

    @Test
    fun an20SliderHasItsNameValueAndAdjustAction() {
        var preamp by mutableFloatStateOf(-3f)
        show {
            CustomSlider(
                value = preamp,
                onValueChange = { preamp = it },
                valueRange = -60f..10f,
                label = "Preamp",
                stateText = { "%.1f dB".format(it) },
                modifier = Modifier.fillMaxWidth()
            )
        }
        val slider = rule.onNodeWithContentDescription("Preamp")
        slider.assert(hasStateDescription("-3.0 dB"))
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.ProgressBarRangeInfo, ProgressBarRangeInfo(-3f, -60f..10f)))
            .assert(SemanticsMatcher.keyIsDefined(SemanticsActions.SetProgress))
        slider.performSemanticsAction(SemanticsActions.SetProgress) { it(-12f) }
        assertEquals(-12f, preamp)
        slider.assert(hasStateDescription("-12.0 dB"))
    }

    @Test
    fun an20ParameterRowNamesItsSliderAndField() {
        show { ParameterRow("Threshold", "dB", -6f, -40f..0f, set = {}, maxDecimals = 1) }
        // The slider and the typed field both carry the row's title and the value with its unit
        val named = rule.onAllNodes(hasContentDescription("Threshold")).fetchSemanticsNodes()
        assertEquals("slider and field", 2, named.size)
        named.forEach { assertEquals("-6.0 dB", it.config[SemanticsProperties.StateDescription]) }
    }

    @Test
    fun an20MeterReadsItsLevelInSteps() {
        var level by mutableFloatStateOf(0.25f)
        var clipping by mutableStateOf(false)
        show {
            HorizontalMeterBar({
                level
            }, androidx.compose.ui.graphics.Color.Green, Modifier.width(200.dp), clipping = { clipping }, label = "USB 1 level")
        }
        val meter = rule.onNodeWithContentDescription("USB 1 level")
        meter.assert(hasStateDescription("-12 dB"))
        level = 0.26f // the same 3 dB step: nothing new to say
        rule.waitForIdle()
        meter.assert(hasStateDescription("-12 dB"))
        level = 0.5f
        rule.waitForIdle()
        meter.assert(hasStateDescription("-6 dB"))
        clipping = true
        rule.waitForIdle()
        meter.assert(hasStateDescription("Clipping"))
    }

    @Test
    fun an20MeterReadoutSteps() {
        assertEquals(MeterReadout("0 dB", 1f), MeterReadout.of(1f, false))
        assertEquals("Silent", MeterReadout.of(0f, false).text)
        assertEquals("Silent", MeterReadout.of(0.0005f, false).text)
        assertEquals("-60 dB", MeterReadout.of(0.001f, false).text)
        assertEquals("-6 dB", MeterReadout.of(0.5f, false).text)
        assertEquals("Clipping", MeterReadout.of(0.1f, true).text)
    }

    @Test
    fun an20CpuMeterIsOneNode() {
        show { CpuMeter(1, 42) }
        rule.onNodeWithContentDescription("Core 1 load").assert(hasStateDescription("42%"))
        assertTrue("no separate \"C1:\" node", rule.onAllNodes(androidx.compose.ui.test.hasText("C1:")).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun an20TogglesSayTheirState() {
        var bypass by mutableStateOf(false)
        var active by mutableStateOf(true)
        show {
            Column {
                SidebarIconButton(DspiIcons.xmark, bypass, "Bypass Master EQ", { bypass = !bypass }, isToggle = true)
                SidebarIconButton(DspiIcons.gear, false, "Settings", {})
                BypassCheckbox(isActive = active, isEnabled = true, onToggle = { active = !active }, label = "Band 1")
            }
        }
        val toggle = rule.onNode(hasContentDescription("Bypass Master EQ") and isToggleable())
        toggle.assertIsOff().assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Switch))
        toggle.performClick()
        toggle.assertIsOn()
        rule.onNodeWithContentDescription("Settings").assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
        val band = rule.onNodeWithContentDescription("Band 1")
        band.assert(SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.On))
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Checkbox))
        band.performClick()
        band.assert(SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.Off))
    }

    @Test
    fun an20SegmentsAreSelectableTabs() {
        var peq by mutableStateOf(true)
        show { SegmentedControl(listOf(SegmentTab("PEQ", peq) { peq = true }, SegmentTab("XO", !peq) { peq = false })) }
        rule.onNodeWithText("PEQ").assertIsSelected().assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab))
        rule.onNodeWithText("XO").performClick()
        rule.onNodeWithText("XO").assertIsSelected()
    }

    @Test
    fun an20CompactFieldOpensItsEditorFromTalkBack() {
        show { CompactGainField(-1.5f, {}, {}, title = "Output gain") }
        val field = rule.onNodeWithContentDescription("Output gain")
        field.assert(hasStateDescription("-1.5 dB")).assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
        // The editor's cursor blinks forever: no idle with the clock on auto
        rule.mainClock.autoAdvance = false
        field.performSemanticsAction(SemanticsActions.OnClick)
        repeat(5) {
            org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
            rule.mainClock.advanceTimeBy(100)
        }
        rule.onNodeWithText("OK").assertExists()
    }

    @Test
    fun an20AtfPassesOnTheSharedControls() {
        show {
            Column {
                CustomSlider(0.5f, {}, Modifier.fillMaxWidth(), label = "Volume")
                ParameterRow("Threshold", "dB", -6f, -40f..0f, set = {})
                HorizontalMeterBar(0.3f, androidx.compose.ui.graphics.Color.Green, label = "OUT1 level")
                SidebarIconButton(DspiIcons.xmark, false, "Bypass Master EQ", {}, isToggle = true)
                BypassCheckbox(isActive = true, isEnabled = true, onToggle = {}, label = "Band 1")
                CompactGainField(0f, {}, {}, title = "Gain")
                CpuMeter(0, 12)
            }
        }
        rule.waitForIdle()
        assertTrue("ATF sees the Compose nodes", AtfChecks.nodeCount(view) > 8)
        AtfChecks.assertPass(view, "shared controls")
    }

    /** The checker is not blind: a 10 dp unnamed button fails it. */
    @Test
    fun an20AtfCatchesATinyUnnamedButton() {
        show { Box(Modifier.size(10.dp).clickable {}) }
        rule.waitForIdle()
        val errors = AtfChecks.results(view)
        assertTrue("touch target flagged: $errors", errors.any { it.startsWith("TouchTargetSizeCheck") })
        assertTrue("missing label flagged: $errors", errors.any { it.startsWith("SpeakableTextPresentCheck") })
    }
}
