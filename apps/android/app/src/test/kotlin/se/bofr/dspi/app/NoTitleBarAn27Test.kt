package se.bofr.dspi.app

import android.app.Application
import android.view.View
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
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

/**
 * AN27: no title bar. AppScaffold over a real demo bridge, with window insets as the
 * system hands them to an edge-to-edge window (targetSdk 35+). mdpi, so 1 dp = 1 px.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class NoTitleBarAn27Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private var session: DemoSession? = null

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun show(strip: ConnectionStrip, insets: WindowInsetsCompat) {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        // On Home, Compose does not go idle with the clock on auto (60 s timeout), so drive the clock
        rule.mainClock.autoAdvance = false
        lateinit var view: View
        rule.setContent {
            view = LocalView.current
            AppScaffold(bridge, strip, onExitDemo = {}, onRequestPermission = {})
        }
        rule.runOnUiThread { ViewCompat.dispatchApplyWindowInsets(view, insets) }
        // Past the NavHost's opening crossfade
        rule.mainClock.advanceTimeBy(3_000)
    }

    private fun insets(status: Insets, nav: Insets = Insets.NONE, cutout: Insets = Insets.NONE) = WindowInsetsCompat.Builder()
        .setInsets(WindowInsetsCompat.Type.statusBars(), status)
        .setInsets(WindowInsetsCompat.Type.navigationBars(), nav)
        .setInsets(WindowInsetsCompat.Type.displayCutout(), cutout)
        .build()

    private fun stripBounds(): Rect {
        val b = rule.onNodeWithTag(CONNECTION_STRIP_TAG, useUnmergedTree = true).getBoundsInRoot()
        return Rect(b.left.value, b.top.value, b.right.value, b.bottom.value)
    }

    /** The topmost and leftmost edges of all visible text (text scrolled out of view has empty bounds). */
    private fun textEdges(): Pair<Float, Float> {
        val texts = rule.onRoot(useUnmergedTree = true).fetchSemanticsNode().let { root ->
            generateSequence(listOf(root)) { level -> level.flatMap { it.children }.ifEmpty { null } }
                .flatten()
                .filter { it.config.contains(SemanticsProperties.Text) && !it.boundsInRoot.isEmpty }
                .toList()
        }
        assertTrue("the screen shows text", texts.isNotEmpty())
        return texts.minOf { it.boundsInRoot.top } to texts.minOf { it.boundsInRoot.left }
    }

    @Test
    fun homeStartsAtTheStripDirectlyBelowTheStatusBar() {
        show(ConnectionStrip.NoDevice, insets(status = Insets.of(0, 48, 0, 0), nav = Insets.of(0, 0, 0, 24)))
        val strip = stripBounds()
        assertEquals("no bar above the strip, and the strip clears the status bar", 48f, strip.top, 0f)
        assertEquals(0f, strip.left, 0f)
        assertEquals(400f, strip.right, 0f)
        rule.onNodeWithText("DSPi").assertDoesNotExist()
        assertTrue("nothing above the strip", textEdges().first >= 48f)
    }

    @Test
    fun demoModeStillSaysSoWithoutATitle() {
        show(ConnectionStrip.Demo, insets(status = Insets.of(0, 48, 0, 0)))
        assertEquals(48f, stripBounds().top, 0f)
        val inStrip = hasAnyAncestor(hasTestTag(CONNECTION_STRIP_TAG))
        rule.onNode(hasText(ConnectionStrip.Demo.text) and inStrip).assertIsDisplayed()
        rule.onNode(hasText("Exit demo") and inStrip).assertIsDisplayed()
        rule.onNodeWithText("DSPi — Demo (no device)").assertDoesNotExist()
    }

    @Test
    fun withAnOpenUsbDspiTheSlimStripClearsTheStatusBar() {
        // WP AN7: the strip stays with a DSPi open; it holds the device list
        show(ConnectionStrip.Usb, insets(status = Insets.of(0, 48, 0, 0)))
        assertEquals(48f, stripBounds().top, 0f)
        assertTrue("first text below the status bar", textEdges().first >= 48f)
    }

    @Test
    @Config(qualifiers = "w800dp-h400dp-land-mdpi")
    fun landscapeKeepsTheStripOffTheCutoutAndTheNavigationBar() {
        show(
            ConnectionStrip.Demo,
            insets(status = Insets.of(0, 24, 0, 0), nav = Insets.of(0, 0, 60, 0), cutout = Insets.of(100, 0, 0, 0))
        )
        val strip = stripBounds()
        assertEquals(24f, strip.top, 0f)
        assertEquals("off the cutout", 100f, strip.left, 0f)
        assertEquals("off the navigation bar", 740f, strip.right, 0f)
        assertTrue("screen text stays off the cutout too", textEdges().second >= 100f)
    }

    @Test
    fun aTopCutoutTallerThanTheStatusBarWins() {
        show(ConnectionStrip.Demo, insets(status = Insets.of(0, 24, 0, 0), cutout = Insets.of(0, 80, 0, 0)))
        assertEquals(80f, stripBounds().top, 0f)
    }
}
