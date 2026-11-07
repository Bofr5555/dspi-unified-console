package se.bofr.dspi.feature.home

import androidx.compose.runtime.CompositionLocalProvider
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DangerousOp
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_SPDIF
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_USB

/** WP AN35: the input strips follow the host format (M25), refreshed when Android's outputs change. */
@OptIn(ExperimentalCoroutinesApi::class)
class HostStripsWpAn35Test {
    /** A host whose outputs change on demand, as `AudioDeviceCallback` reports. */
    private class FakeHost(var count: Int?) : HostChannelSource {
        val watchers = mutableListOf<() -> Unit>()

        override fun channelCount() = count

        override fun watch(onChange: () -> Unit): AutoCloseable {
            watchers += onChange
            return AutoCloseable { watchers -= onChange }
        }

        fun change(to: Int?) {
            count = to
            watchers.toList().forEach { it() }
        }
    }

    private suspend fun TestScope.connectedBridge(): DspiBridgeImpl {
        val core = FakeDspiCore.seeded()
        val d = StandardTestDispatcher(testScheduler)
        val b = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        check(b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID))
        runCurrent()
        return b
    }

    @Test
    fun wpAn35TheStripFollowsTheHostCount() = runTest {
        val b = connectedBridge()
        val device = b.numInputChannels
        assertTrue("the fixture has more than one pair", device >= 4)
        val host = FakeHost(4)
        val seen = mutableListOf<Int>()
        CompositionHarness().use { h ->
            h.setContent { CompositionLocalProvider(LocalHostChannelSource provides host) { seen += stripInputCount(b) } }
            assertEquals("the host's 4 channels", 4, seen.last())
            assertEquals("one AudioDeviceCallback while shown", 1, host.watchers.size)

            host.change(2)
            h.frame()
            assertEquals("a stereo host format", 2, seen.last())

            host.change(null)
            h.frame()
            assertEquals("no DSPi output: the device's report", minOf(device, maxOf(2, b.activeInputChannels)), seen.last())

            host.change(8)
            h.frame()
            assertEquals("never above the device's inputs", minOf(8, device), seen.last())

            // S/PDIF is stereo whatever the host sends
            b.setInputSource(b.dangerousOps.confirm(DangerousOp.HARDWARE_APPLY), SOURCE_SPDIF)
            runCurrent()
            h.frame()
            assertEquals(2, seen.last())
            b.setInputSource(b.dangerousOps.confirm(DangerousOp.HARDWARE_APPLY), SOURCE_USB)
            runCurrent()
            h.frame()
            assertEquals(minOf(8, device), seen.last())
        }
        assertEquals("the callback is unregistered with the screen", 0, host.watchers.size)
    }

    @Test
    fun wpAn35PairsFollowTheStrips() {
        // Unknown active count or no inputs: no strips
        assertEquals(0, HostStrips.count(8, -1, SOURCE_USB, 8, 2))
        assertEquals(0, HostStrips.count(8, 2, SOURCE_USB, 0, 2))
        // The host keeps 8 strips while the device idles to stereo
        assertEquals(8, HostStrips.count(8, 2, SOURCE_USB, 8, 2))
        assertTrue(HostStrips.pairAvailable(6, 8))
        assertTrue(HostStrips.pairAvailable(7, 8))
        assertFalse(HostStrips.pairAvailable(2, 2))
        assertFalse(HostStrips.pairAvailable(-1, 8))
        // links: one bit per pair, only for a pair laid out whole
        assertTrue(HostStrips.linked(2, 4, 0b10))
        assertTrue(HostStrips.linked(3, 4, 0b10))
        assertFalse(HostStrips.linked(2, 2, 0b10))
        assertFalse(HostStrips.linked(0, 4, 0b10))
    }
}
