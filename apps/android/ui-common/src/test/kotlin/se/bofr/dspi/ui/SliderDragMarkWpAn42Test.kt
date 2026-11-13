package se.bofr.dspi.ui

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * WP AN42: a slider drag holds the app's drag mark, so its writes are one undo
 * step. The mark starts on the grab and ends after the release's commit, on a
 * cancelled gesture, when the slider leaves the screen and when it is disabled
 * mid-drag (read-only). [DragMarker] tells the core once for overlapping drags.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class SliderDragMarkWpAn42Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val log = mutableListOf<String>()
    private val marker = DragMarker { log += "mark $it" }
    private var shown by mutableStateOf(true)
    private var writes by mutableStateOf(true)

    private fun show() = rule.setContent {
        CompositionLocalProvider(LocalDragMarker provides marker, LocalWritesAllowed provides writes) {
            DspiTheme(darkTheme = true) {
                if (shown) {
                    CustomSlider(
                        value = 0.5f,
                        onValueChange = { if (log.lastOrNull() != "change") log += "change" },
                        onEditingChanged = { if (!it) log += "commit" },
                        modifier = Modifier.width(300.dp).testTag("slider")
                    )
                }
            }
        }
    }

    private fun grabAndMove() {
        rule.onNodeWithTag("slider").performTouchInput {
            down(center)
            repeat(6) { moveBy(Offset(15f, 0f)) }
        }
        rule.waitForIdle()
    }

    @Test
    fun wpAn42ADragIsMarkedAndTheCommitComesFirst() {
        show()
        grabAndMove()
        rule.onNodeWithTag("slider").performTouchInput { up() }
        rule.waitForIdle()
        assertEquals(listOf("mark true", "change", "commit", "mark false"), log)
        assertFalse(marker.active)
    }

    @Test
    fun wpAn42ACancelledGestureEndsTheMark() {
        show()
        grabAndMove()
        rule.onNodeWithTag("slider").performTouchInput { cancel() }
        rule.waitForIdle()
        assertEquals("mark false", log.last())
        assertFalse(marker.active)
    }

    @Test
    fun wpAn42LeavingTheScreenMidDragEndsTheMark() {
        show()
        grabAndMove()
        assertTrue(marker.active)
        shown = false
        rule.waitForIdle()
        assertEquals("mark false", log.last())
        assertFalse(marker.active)
    }

    @Test
    fun wpAn42ReadOnlyMidDragEndsTheMark() {
        show()
        grabAndMove()
        writes = false
        rule.waitForIdle()
        assertEquals("mark false", log.last())
        assertFalse(marker.active)
    }

    @Test
    fun wpAn42OverlappingDragsAreOneMarkAndEndAllEndsIt() {
        val sent = mutableListOf<Boolean>()
        val m = DragMarker { sent += it }
        val a = Any()
        val b = Any()
        m.begin(a)
        m.begin(b)
        m.begin(a)
        m.end(a)
        assertEquals(listOf(true), sent)
        m.end(b)
        assertEquals(listOf(true, false), sent)
        m.end(b) // nothing held: nothing sent
        m.begin(a)
        m.endAll() // the app stopped
        m.end(a) // the control's own late end
        m.endAll()
        assertEquals(listOf(true, false, true, false), sent)
    }
}
