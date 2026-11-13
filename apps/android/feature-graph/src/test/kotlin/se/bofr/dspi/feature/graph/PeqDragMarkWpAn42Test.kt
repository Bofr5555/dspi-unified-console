package se.bofr.dspi.feature.graph

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiConstants.FILTER_FLAT
import se.bofr.dspi.core.DspiConstants.FILTER_PEAKING
import se.bofr.dspi.core.FilterParams
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.DragMarker

/**
 * WP AN42: a dot drag on the graph is one undo step. The host hears the start
 * once, and the end once, after the drag's commit (so the commit joins the
 * step), also when the gesture is cancelled, the editor is reset (editing
 * off) or disposed mid-drag. A click or a wheel edit marks nothing.
 */
class PeqDragMarkWpAn42Test {
    private class MarkHost : PeqGraphEditorHost {
        override val selection = PeqGraphSelection()
        val log = mutableListOf<String>()

        override fun commitGraphBands(channel: Int, changes: List<Pair<Int, FilterParams>>) {
            log += "commit"
        }

        override fun sendGraphBandsToDevice(channel: Int, changes: List<Pair<Int, FilterParams>>) {
            if (log.lastOrNull() != "send") log += "send"
        }

        override fun setGraphBandBypass(channel: Int, bands: List<Int>, bypass: Boolean) {
            log += "bypass"
        }

        override fun showLive(channel: Int, band: Int, p: FilterParams) {}

        override fun endLive(channel: Int) {}

        override fun markDrag(dragging: Boolean) {
            log += "mark $dragging"
        }
    }

    private val g = PeqGraphGeometry(800.0, 300.0, 20.0, 20000.0, 25.0, -25.0)

    private fun config(editing: Boolean = true): PeqEditorConfig {
        val bands = MutableList(10) { FilterTypes.off() }
        bands[2] = FilterTypes.band(FILTER_PEAKING, 1000f, 1.5f, 3f)
        return PeqEditorConfig(
            channel = if (editing) 0 else null,
            bands = bands,
            minFreq = 20.0,
            maxFreq = 20000.0,
            availableTypes = FilterTypes.PEQ_TYPES.filter { it != FILTER_FLAT }.toSet(),
            bypassSupported = true
        )
    }

    private fun rig(host: MarkHost): PeqGraphEditor = PeqGraphEditor(host, FakeClock()).apply {
        resize(800.0, 300.0)
        apply(config())
    }

    private fun PeqGraphEditor.startDrag() {
        val dot = PeqPoint(g.x(1000.0), g.y(3.0))
        pointerPressed(dot, PeqMods.NONE, 1)
        for (i in 1..5) pointerDragged(PeqPoint(dot.x + 10.0 * i, dot.y - 4.0 * i), PeqMods.NONE)
    }

    @Test
    fun wpAn42ADragIsMarkedAndItsCommitComesBeforeTheEnd() {
        val host = MarkHost()
        val e = rig(host)
        e.startDrag()
        assertEquals(listOf("mark true", "send"), host.log)
        e.pointerReleased(PeqPoint(g.x(2000.0), g.y(9.0)))
        assertEquals(listOf("mark true", "send", "commit", "mark false"), host.log)
    }

    @Test
    fun wpAn42ACancelledDragEndsItsMark() {
        val host = MarkHost()
        val e = rig(host)
        e.startDrag()
        e.pointerCanceled()
        assertEquals("mark false", host.log.last())
        assertEquals(1, host.log.count { it == "mark false" })
    }

    @Test
    fun wpAn42ATwoFingerGestureMidDragEndsItsMark() {
        val host = MarkHost()
        val e = rig(host)
        e.startDrag()
        e.abandonPress()
        assertEquals("mark false", host.log.last())
    }

    @Test
    fun wpAn42DisposingMidDragCommitsThenEndsTheMark() {
        val host = MarkHost()
        val e = rig(host)
        e.startDrag()
        e.dispose()
        assertEquals(listOf("commit", "mark false"), host.log.takeLast(2))
    }

    @Test
    fun wpAn42EditingTurnedOffMidDragEndsTheMark() {
        val host = MarkHost()
        val e = rig(host)
        e.startDrag()
        e.apply(config(editing = false))
        assertEquals(1, host.log.count { it == "mark false" })
        e.dispose()
        assertEquals("ended once", 1, host.log.count { it == "mark false" })
    }

    @Test
    fun wpAn42AClickMarksNothing() {
        val host = MarkHost()
        val e = rig(host)
        val dot = PeqPoint(g.x(1000.0), g.y(3.0))
        e.pointerPressed(dot, PeqMods.NONE, 1)
        e.pointerReleased(dot)
        assertTrue(host.log.none { it.startsWith("mark") })
    }

    /** The bridge's host holds the app's mark under itself: true at the start, false at the end, in call order with the writes. */
    @OptIn(ExperimentalCoroutinesApi::class)
    @Test
    fun wpAn42TheBridgeHostMarksThroughTheAppsMarker() = runTest {
        val core = FakeDspiCore()
        val d = kotlinx.coroutines.test.StandardTestDispatcher(testScheduler)
        val bridge =
            DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, linkStore = InMemoryLinkStore(), clock = { testScheduler.currentTime })
        bridge.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        val marks = mutableListOf<Boolean>()
        val marker = DragMarker {
            marks += it
            bridge.setDragging(it)
        }
        val host = BridgePeqEditorHost(bridge, PeqGraphSelection(), backgroundScope, marker)
        val p = FilterTypes.band(FILTER_PEAKING, 1200f, 1f, 4f)
        host.markDrag(true)
        host.sendGraphBandsToDevice(0, listOf(2 to p))
        runCurrent()
        host.commitGraphBands(0, listOf(2 to p))
        host.markDrag(false)
        host.markDrag(false) // a second end is nothing
        runCurrent()
        assertEquals(listOf(true, false), marks)
        assertFalse(marker.active)
        val order = core.calls.filter { it.startsWith("setDragging") || it.startsWith("setFilter") }
        assertEquals("setDragging(true)", order.first())
        assertEquals("setDragging(false)", order.last())
        assertTrue(bridge.undoState.value.canUndo)
        bridge.undo()
        assertFalse("one step", bridge.undoState.value.canUndo)
    }
}
