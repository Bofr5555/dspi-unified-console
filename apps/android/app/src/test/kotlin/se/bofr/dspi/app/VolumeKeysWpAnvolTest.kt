package se.bofr.dspi.app

import android.view.KeyEvent
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.bridge.InMemoryLinkStore
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP ANVOL: the volume buttons on the DSPi's User Volume. The step (1 dB on the whole-dB grid),
 * the clamp at -60 and 0 dB, the pacing of a held key, mute, and when the keys are left to
 * Android: the setting off, the app in the background, no DSPi, a read-only connection.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class VolumeKeysWpAnvolTest {
    private class Host(var db: Float = -20f) : VolumeKeyHost {
        override var enabled = true
        override var foreground = true
        override var deviceWritable = true
        override val userVolumeDb get() = db
        var muted = false
        override val userMute get() = muted
        val writes = mutableListOf<Float>()

        override fun setUserVolume(db: Float) {
            writes += db
        }

        override fun setUserMute(muted: Boolean) {
            this.muted = muted
        }
    }

    private fun TestScope.controller(host: VolumeKeyHost) = VolumeKeyController(backgroundScope, host) { testScheduler.currentTime }

    /** Past the pacing and the hold (the controller and the bridge run in backgroundScope, which advanceUntilIdle skips). */
    private fun TestScope.settle() {
        testScheduler.advanceTimeBy(VolumeKeyController.HOLD_MS * 2)
        runCurrent()
    }

    private fun VolumeKeyController.press(key: Int, repeat: Int = 0) = onKey(key, KeyEvent.ACTION_DOWN, repeat)

    private fun VolumeKeyController.release(key: Int) = onKey(key, KeyEvent.ACTION_UP, 0)

    private val up = KeyEvent.KEYCODE_VOLUME_UP
    private val down = KeyEvent.KEYCODE_VOLUME_DOWN

    @Test
    fun wpAnvolOneDbOnTheWholeDbGridHeldToTheRange() {
        assertEquals(-22f, VolumeKeyController.nextLevel(-23f, +1))
        assertEquals(-24f, VolumeKeyController.nextLevel(-23f, -1))
        // a slider level between steps goes to the next whole dB either way
        assertEquals(-23f, VolumeKeyController.nextLevel(-23.4f, +1))
        assertEquals(-24f, VolumeKeyController.nextLevel(-23.4f, -1))
        // the firmware's 8.8 fixed-point echo of -23 dB counts as -23
        assertEquals(-22f, VolumeKeyController.nextLevel(-23.0039f, +1))
        assertEquals(-24f, VolumeKeyController.nextLevel(-22.9961f, -1))
        // the core's and the firmware's range
        assertEquals(0f, VolumeKeyController.nextLevel(0f, +1))
        assertEquals(0f, VolumeKeyController.nextLevel(-0.5f, +1))
        assertEquals(-60f, VolumeKeyController.nextLevel(-60f, -1))
        assertEquals(-60f, VolumeKeyController.nextLevel(-59.5f, -1))
        assertEquals(0f, VolumeKeyController.nextLevel(Float.NaN, +1))
    }

    @Test
    fun wpAnvolPressesStepAndAreHandled() = runTest {
        val h = Host(-20f)
        val c = controller(h)
        assertTrue(c.press(up))
        assertTrue(c.release(up))
        assertEquals(listOf(-19f), h.writes)
        assertEquals(-19f, c.feedback.value?.db)
        testScheduler.advanceTimeBy(VolumeKeyController.SEND_INTERVAL_MS * 3)
        runCurrent()
        // the next press counts from the level asked for, before the device's echo
        assertTrue(c.press(down))
        assertTrue(c.release(down))
        settle()
        assertEquals(listOf(-19f, -20f), h.writes)
        assertEquals(-20f, c.feedback.value?.db)
    }

    @Test
    fun wpAnvolAtTheEndsAPressIsHandledShownAndNotSent() = runTest {
        val top = Host(0f)
        val c = controller(top)
        assertTrue("never left to Android's volume at the top", c.press(up))
        c.release(up)
        settle()
        assertEquals(emptyList<Float>(), top.writes)
        assertEquals(0f, c.feedback.value?.db)

        val bottom = Host(-60f)
        val d = controller(bottom)
        assertTrue(d.press(down))
        settle()
        assertEquals(emptyList<Float>(), bottom.writes)
        assertEquals(-60f, d.feedback.value?.db)
    }

    @Test
    fun wpAnvolAHeldKeyIsPacedAndItsLastLevelGoesOut() = runTest {
        val h = Host(-40f)
        val c = controller(h)
        // Android's key repeat: every 50 ms; 20 presses in one second
        for (i in 0 until 20) {
            assertTrue(c.press(up, repeat = i))
            testScheduler.advanceTimeBy(50)
            runCurrent()
        }
        assertTrue(c.release(up))
        settle()
        assertEquals(-20f, c.feedback.value?.db)
        assertEquals("the last level always goes out", -20f, h.writes.last())
        assertTrue("at most one write per 100 ms: ${h.writes}", h.writes.size <= 11)
        assertEquals(-39f, h.writes.first())
        assertEquals("levels only go up while up is held", h.writes.sorted(), h.writes)
    }

    @Test
    fun wpAnvolAHeldKeyStopsAtTheTop() = runTest {
        val h = Host(-3f)
        val c = controller(h)
        for (i in 0 until 10) {
            c.press(up, repeat = i)
            testScheduler.advanceTimeBy(50)
            runCurrent()
        }
        c.release(up)
        settle()
        assertEquals(0f, h.writes.last())
        assertTrue(h.writes.all { it <= 0f })
        assertEquals(0f, c.feedback.value?.db)
    }

    @Test
    fun wpAnvolMuteKeyTogglesTheUserMuteOncePerPress() = runTest {
        val h = Host(-20f)
        val c = controller(h)
        val mute = KeyEvent.KEYCODE_VOLUME_MUTE
        assertTrue(c.press(mute))
        assertTrue(c.press(mute, repeat = 1))
        assertTrue(c.release(mute))
        assertTrue(h.userMute)
        assertEquals(VolumeKeyFeedback(-20f, true, 1), c.feedback.value)
        c.press(mute)
        c.release(mute)
        assertFalse(h.userMute)
        assertEquals(emptyList<Float>(), h.writes)
    }

    @Test
    fun wpAnvolInactiveTheKeysAreLeftToAndroid() = runTest {
        val cases = listOf<Pair<String, (Host) -> Unit>>(
            "setting off" to { it.enabled = false },
            "app in the background" to { it.foreground = false },
            "no DSPi or a read-only connection" to { it.deviceWritable = false }
        )
        for ((why, off) in cases) {
            val h = Host(-20f).also(off)
            val c = controller(h)
            for (key in VolumeKeyController.KEYS) {
                assertFalse(why, c.press(key))
                assertFalse("$why: its key-up too", c.release(key))
            }
            settle()
            assertEquals(why, emptyList<Float>(), h.writes)
            assertFalse(why, h.userMute)
            assertNull(why, c.feedback.value)
        }
    }

    @Test
    fun wpAnvolOtherKeysAndAKeyUpWhoseDownWasNotHandledPass() = runTest {
        val h = Host(-20f)
        val c = controller(h)
        assertFalse(c.press(KeyEvent.KEYCODE_ENTER))
        assertFalse(c.release(KeyEvent.KEYCODE_ENTER))
        // the down came while inactive, the up after: the up is Android's
        h.foreground = false
        assertFalse(c.press(up))
        h.foreground = true
        assertFalse(c.release(up))
        // the down was handled, the app went inactive before the up: the up is still ours
        assertTrue(c.press(up))
        h.enabled = false
        assertTrue(c.release(up))
    }

    @Test
    fun wpAnvolTheDeviceGoingAwayMidHoldStopsTheWrites() = runTest {
        val h = Host(-30f)
        val c = controller(h)
        c.press(up)
        c.press(up, repeat = 1)
        c.press(up, repeat = 2)
        h.deviceWritable = false
        settle()
        assertEquals(listOf(-29f), h.writes)
    }

    // The keys on the real bridge over the fake core: the device's User Volume moves, the bridge's
    // state (the Home slider's) follows, and with no DSPi or a DSPi Link viewer nothing is handled.
    private fun TestScope.bridge(core: FakeDspiCore): DspiBridgeImpl {
        val d = StandardTestDispatcher(testScheduler)
        return DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, linkStore = InMemoryLinkStore(), clock = {
            testScheduler.currentTime
        })
    }

    @Test
    fun wpAnvolOnTheBridgeTheDevicesUserVolumeMoves() = runTest {
        val core = FakeDspiCore()
        val b = bridge(core)
        var setting = true
        val c = controller(BridgeVolumeKeyHost({ b }, { setting }, { true }))
        assertFalse("no DSPi: Android's volume", c.press(down))
        c.release(down)

        b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID)
        runCurrent()
        val start = b.userVolumeDB
        assertTrue(c.press(down))
        c.release(down)
        settle()
        assertEquals(VolumeKeyController.nextLevel(start, -1), core.getState().userVolumeDb)
        assertEquals("the slider's state follows", core.getState().userVolumeDb, b.userVolumeDB)

        assertTrue(c.press(KeyEvent.KEYCODE_VOLUME_MUTE))
        c.release(KeyEvent.KEYCODE_VOLUME_MUTE)
        settle()
        assertTrue(core.getState().userMute)

        // a DSPi Link viewer: read-only, the keys are Android's
        b.setRemoteReadOnly(true)
        runCurrent()
        val calls = core.callCount("setUserVolume")
        assertFalse(c.press(down))
        settle()
        assertEquals(calls, core.callCount("setUserVolume"))
        b.setRemoteReadOnly(false)
        runCurrent()

        setting = false
        assertFalse("setting off", c.press(down))
    }
}
