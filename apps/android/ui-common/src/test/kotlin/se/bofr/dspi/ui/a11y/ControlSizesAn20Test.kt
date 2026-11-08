package se.bofr.dspi.ui.a11y

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.ui.BypassCheckbox
import se.bofr.dspi.ui.CustomSlider
import se.bofr.dspi.ui.DspiIcons
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.ParameterRow
import se.bofr.dspi.ui.PillButton
import se.bofr.dspi.ui.PopUpButton
import se.bofr.dspi.ui.SegmentTab
import se.bofr.dspi.ui.SegmentedControl
import se.bofr.dspi.ui.SidebarIconButton
import se.bofr.dspi.ui.SplitActionPill
import se.bofr.dspi.ui.ToolChip
import se.bofr.dspi.ui.ValueField

/**
 * WP AN20 (review c6c, AN20-1): the shared controls keep the laid-out sizes
 * they have on main. The owner's rule of 2026-11-08: accessibility may not
 * grow the phone's layout, so no 48 dp minimum sizes; the semantics stay.
 * The sizes are the review's probe on main (mdpi, 1 px = 1 dp); a change
 * here is a visible layout change and needs the owner's say.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ControlSizesAn20Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun size(tag: String): String {
        val b = rule.onNodeWithTag(tag).getUnclippedBoundsInRoot()
        return "${(b.right - b.left).value.toInt()}x${(b.bottom - b.top).value.toInt()}"
    }

    private val controls: List<Pair<String, @Composable () -> Unit>> = listOf(
        "slider" to { CustomSlider(0.5f, {}, Modifier.width(200.dp)) },
        "icon" to { SidebarIconButton(DspiIcons.gear, false, "Settings", {}) },
        "bypass" to { BypassCheckbox(isActive = true, isEnabled = true, onToggle = {}) },
        "chip" to { ToolChip("1", true, true, Modifier.width(60.dp)) {} },
        "pill" to { PillButton("Link", {}) },
        "split" to { SplitActionPill("Save", false, {}, "Load", false, {}) },
        "segments" to {
            SegmentedControl(listOf(SegmentTab("PEQ", true) {}, SegmentTab("XO", false) {}), Modifier.fillMaxWidth())
        },
        "field" to { ValueField("Q", 1f, 52.dp) {} },
        "popup" to { PopUpButton(listOf("A", "B"), "A", { it }, {}) },
        "prow" to { ParameterRow("Threshold", "dB", -6f, -40f..0f, set = {}, modifier = Modifier.width(300.dp)) }
    )

    @Test
    fun an20ControlsKeepMainsLaidOutSizes() {
        // Each control in a bare Box: the Box takes the control's laid-out size, whatever its own
        // modifiers do to its semantics bounds
        rule.setContent { DspiTheme(darkTheme = true) { Column { controls.forEach { (tag, c) -> Box(Modifier.testTag(tag)) { c() } } } } }
        rule.waitForIdle()
        val measured = controls.associate { (tag, _) -> tag to size(tag) }
        measured.forEach { (tag, s) -> println("PROBE $tag $s") }
        assertEquals(MAIN_SIZES, measured)
    }

    companion object {
        /** Measured on main 35e2e3cc with this test (review c6c's probe-main.log matches, its split pill had other labels). */
        val MAIN_SIZES = mapOf(
            "slider" to "200x32",
            "icon" to "44x44",
            "bypass" to "36x36",
            "chip" to "60x36",
            "pill" to "43x26",
            "split" to "95x26",
            "segments" to "400x26",
            "field" to "76x26",
            "popup" to "38x32",
            "prow" to "300x62"
        )
    }
}
