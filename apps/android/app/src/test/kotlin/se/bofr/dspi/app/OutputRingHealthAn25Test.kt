package se.bofr.dspi.app

import android.app.Application
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import se.bofr.dspi.core.FillBand
import se.bofr.dspi.core.OutputFill
import se.bofr.dspi.core.RingHealth
import se.bofr.dspi.core.StatsTick
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.analysis.BufferFillThresholds
import se.bofr.dspi.feature.analysis.FillHealth
import se.bofr.dspi.feature.analysis.RING_HEALTH_LABEL
import se.bofr.dspi.feature.analysis.StatsView
import se.bofr.dspi.feature.analysis.statsFillTag

/** AN25: System Statistics over a real demo session. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w900dp-h3000dp-land-mdpi")
class OutputRingHealthAn25Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun show(core: FakeDspiCore) {
        val bridge = startDemoSession(scope, "test", core = core).also { session = it }.bridge
        rule.setContent { StatsView(bridge) }
        rule.waitUntil(WAIT_MS) { rule.onAllNodesWithTag(statsFillTag(0)).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun fill(series: Int): Pair<String, String?> {
        val c = rule.onNodeWithTag(statsFillTag(series)).fetchSemanticsNode().config
        return c[SemanticsProperties.Text].single().text to c[SemanticsProperties.StateDescription]
    }

    @Test
    fun an25DemoDeviceShowsTheSectionAndRingScaledBands() {
        show(FakeDspiCore.demo())
        rule.waitUntil(WAIT_MS) { rule.onAllNodesWithText(RING_HEALTH_LABEL).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText("96 frames (2 ms)").assertExists()
        rule.onNodeWithText("Underrun Episodes").assertExists()
        rule.onNodeWithText("Refused Blocks").assertExists()
        rule.onNodeWithText("72 frames").assertExists()
        rule.onNodeWithText("Worst Input Gap").assertExists()
        rule.onNodeWithText("Worst USB Gap").assertExists()
        assertEquals(2, rule.onAllNodesWithText("1000 µs").fetchSemanticsNodes().size)
        rule.onNodeWithText("Over-Budget Steps").assertExists()

        // Around the 96-frame target a ring is healthy where the fixed bands would warn
        rule.waitUntil(WAIT_MS) { fill(0).first.removeSuffix("%").toInt() < 25 }
        val (text, band) = fill(0)
        val pct = text.removeSuffix("%").toInt()
        assertEquals(FillHealth.YELLOW, BufferFillThresholds.SPDIF.health(pct))
        assertEquals("Good", band)
    }

    @Test
    fun an25TheCoresBandsColourTheRows() {
        show(
            object : FakeDspiCore() {
                override fun fetchStatsTick(): StatsTick = super.fetchStatsTick().copy(
                    fills = listOf(
                        OutputFill(18.75, 14, 23, FillBand.GOOD),
                        OutputFill(3.0, 1, 23, FillBand.WARN),
                        OutputFill(0.0, 0, 23, FillBand.BAD),
                        OutputFill(95.0, 14, 96, FillBand.BAD)
                    )
                )
            }
        )
        rule.waitUntil(WAIT_MS) { fill(1).second == "Warning" }
        assertEquals("19%" to "Good", fill(0))
        assertEquals("3%" to "Warning", fill(1))
        assertEquals("0%" to "Bad", fill(2))
        assertEquals("95%" to "Bad", fill(3))
    }

    @Test
    fun an25FirmwareWithoutItHasNoSectionAndKeepsTheFixedBands() {
        val asked = AtomicBoolean(false)
        show(
            object : FakeDspiCore() {
                override fun fetchRingHealth(): RingHealth? {
                    asked.set(true)
                    return null
                }

                override fun fetchStatsTick(): StatsTick = super.fetchStatsTick().copy(fills = emptyList())
            }
        )
        // Past the answer itself: it cannot have been missed
        rule.waitUntil(WAIT_MS) { asked.get() }
        rule.waitForIdle()
        rule.onNodeWithText(RING_HEALTH_LABEL).assertDoesNotExist()
        rule.onNodeWithText("Fill Target").assertDoesNotExist()
        val (text, band) = fill(0)
        assertTrue(text, text.removeSuffix("%").toInt() in 47..53)
        assertEquals("Good", band)
    }

    private companion object {
        const val WAIT_MS = 20_000L
    }
}
