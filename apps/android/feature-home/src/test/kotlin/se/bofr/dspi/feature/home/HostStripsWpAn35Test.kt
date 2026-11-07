package se.bofr.dspi.feature.home

import androidx.compose.runtime.CompositionLocalProvider
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DangerousOp
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.SystemStatus
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_SPDIF
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_USB

/**
 * WP AN35: the input strips follow the host format (M25), refreshed when
 * Android's outputs change, and the bridge mirrors a link over the same count
 * (review c4k F1: one count for the UI and the mirror, macOS numMatrixInputs).
 */
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

    /** The seeded device with nothing playing: it reports its stereo fallback. */
    private class IdleCore(private val f: FakeDspiCore) : DspiCoreApi by f {
        override fun fetchStatus(): SystemStatus = f.fetchStatus().copy(activeInputChannels = 2)
    }

    private suspend fun TestScope.connectedBridge(idle: Boolean = false): DspiBridgeImpl {
        val fake = FakeDspiCore.seeded()
        val core: DspiCoreApi = if (idle) IdleCore(fake) else fake
        val d = StandardTestDispatcher(testScheduler)
        val b = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        check(b.attachDevice(42, fake.serial, DspiConstants.VENDOR_ID))
        runCurrent()
        if (idle) {
            // One status poll, so the bridge has the device's idle report
            b.setUiVisible(true)
            advanceTimeBy(200)
            runCurrent()
            b.setUiVisible(false)
        }
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
            h.setContent {
                CompositionLocalProvider(LocalHostChannelSource provides host) {
                    HostChannelsFeed(b)
                    seen += stripInputCount(b)
                }
            }
            h.frame()
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
        assertNull("and the bridge forgets the host", b.hostInputChannels.value)
    }

    @Test
    fun wpAn35DemoAndLinkGiveTheBridgeNoHostCount() = runTest {
        val b = connectedBridge()
        val host = FakeHost(4)
        CompositionHarness().use { h ->
            h.setContent { CompositionLocalProvider(LocalHostChannelSource provides host) { HostChannelsFeed(b, enabled = false) } }
            assertNull(b.hostInputChannels.value)
            assertEquals(0, host.watchers.size)
        }
    }

    /** Review c4k F1's probe: a pair the idle host shows is one the bridge links and mirrors. */
    @Test
    fun wpAn35AShownPairIsLinkedAndMirroredWhileTheDeviceIdles() = runTest {
        val b = connectedBridge(idle = true)
        assertTrue("the fixture has a second pair", b.numInputChannels >= 4)
        assertEquals("the device idles to stereo", 2, b.activeInputChannels)
        val host = FakeHost(4)
        val strips = mutableListOf<Int>()
        CompositionHarness().use { h ->
            h.setContent {
                CompositionLocalProvider(LocalHostChannelSource provides host) {
                    HostChannelsFeed(b)
                    strips += stripInputCount(b)
                }
            }
            h.frame()
            assertEquals("the host's 4 strips", 4, strips.last())
            val in3 = b.inputAppId(2)
            val in4 = b.inputAppId(3)
            // InputChannelHeader's link toggle and the sidebar ask the same bridge
            assertTrue("IN3/4 can be linked", b.pairAvailable(in3))
            b.setInputLinked(in3, true, -1)
            runCurrent()
            assertTrue(b.isInputLinked(in3))
            assertEquals("the sidebar selects both rows", in4, b.linkedPartner(in3))
            b.setInputPreamp(2, -6f)
            runCurrent()
            assertEquals(-6f, b.inputPreampDB(2), 0.01f)
            assertEquals("IN4 follows IN3", -6f, b.inputPreampDB(3), 0.01f)
        }
    }
}
