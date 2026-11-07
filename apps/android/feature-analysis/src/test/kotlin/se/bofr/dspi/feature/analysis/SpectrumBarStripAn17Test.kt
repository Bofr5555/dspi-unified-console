package se.bofr.dspi.feature.analysis

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.RtaOwnership

/** WP AN17: the bar strip's sizes and the following selection, without a view. */
class SpectrumBarStripAn17Test {
    private val out01 = RtaChannelSelection.of(DspiConstants.RTA_TAP_OUTPUT, listOf(0, 1))

    @Test
    fun an17DrivingDrawsTheOwnSelection() {
        assertNull(RtaChannelSelection.followed(null))
        assertNull(RtaChannelSelection.followed(RtaOwnership(following = false, origin = 0, tap = 0, mask = 0, fftOrder = 0)))
        assertEquals(out01, RtaChannelSelection.drawn(out01, null))
    }

    @Test
    fun an17FollowingDrawsTheDevicesTapAndChannels() {
        val o = RtaOwnership(following = true, origin = 7, tap = DspiConstants.RTA_TAP_INPUT, mask = 0b1010_0000_0000_0101, fftOrder = 9)
        val followed = RtaChannelSelection.followed(o)
        assertEquals(RtaChannelSelection.of(DspiConstants.RTA_TAP_INPUT, listOf(0, 2, 13, 15)), followed)
        assertEquals(followed, RtaChannelSelection.drawn(out01, followed))
        // An unknown writer (origin 0) is still someone else
        assertEquals(
            RtaChannelSelection.of(DspiConstants.RTA_TAP_OUTPUT, listOf(3)),
            RtaChannelSelection.followed(o.copy(origin = 0, tap = DspiConstants.RTA_TAP_OUTPUT, mask = 0b1000))
        )
    }

    @Test
    fun an17CellHeightIsClampedAndThreeQuartersForSeveralChannels() {
        assertEquals(96f, rtaBarStripCellHeight(96.0, single = true))
        assertEquals(72f, rtaBarStripCellHeight(96.0, single = false))
        assertEquals(64f, rtaBarStripCellHeight(10.0, single = true))
        assertEquals(180f, rtaBarStripCellHeight(1000.0, single = false))
        assertEquals(64f, rtaBarStripCellHeight(Double.NaN, single = true))
    }

    @Test
    fun an17DragSharesTheMoveOverTheRows() {
        // One channel: the edge follows the finger
        assertEquals(126.0, rtaBarStripDraggedHeight(96.0, 30f, rows = 1, single = true), 1e-9)
        // Two rows of three-quarter cells: 30 dp of drag is 20 units of preference
        assertEquals(116.0, rtaBarStripDraggedHeight(96.0, 30f, rows = 2, single = false), 1e-9)
        assertEquals(240.0, rtaBarStripDraggedHeight(200.0, 500f, rows = 1, single = true), 1e-9)
        assertEquals(64.0, rtaBarStripDraggedHeight(96.0, -500f, rows = 3, single = false), 1e-9)
    }

    @Test
    fun an17ColumnsNeverExceedTheChannels() {
        assertEquals(1, rtaBarColumns(1, 4))
        assertEquals(2, rtaBarColumns(5, 2))
        assertEquals(3, rtaBarColumns(3, 4))
    }
}
