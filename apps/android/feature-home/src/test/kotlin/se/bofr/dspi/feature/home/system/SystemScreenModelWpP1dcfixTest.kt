package se.bofr.dspi.feature.home.system

import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP P1DCFIX (review item 4): Save on the System screen keeps the hub's
 * warnings about the model it kept (`alink_system_save_report`, as Qt shows
 * them); a clean save and a Reload have none.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SystemScreenModelWpP1dcfixTest {
    private class Rig(scope: TestScope) {
        val core = FakeDspiCore()
        val d = StandardTestDispatcher(scope.testScheduler)
        val bridge = DspiBridgeImpl(core, scope.backgroundScope, ffi = d, main = d, clock = { scope.testScheduler.currentTime })
        val model = SystemScreenModel(bridge, scope)
    }

    private suspend fun TestScope.rig(plainUnit: Boolean): Rig {
        val r = Rig(this)
        assertTrue(r.bridge.attachDevice(42, r.core.serial, DspiConstants.VENDOR_ID))
        advanceUntilIdle()
        r.core.alinkSystem.onHub = true
        r.core.alinkSystem.plainUnit = plainUnit
        r.model.refresh()
        advanceUntilIdle()
        return r
    }

    @Test
    fun p1dcfixSaveShowsTheHubsWarnings() = runTest(timeout = 20.seconds) {
        val r = rig(plainUnit = true)
        assertTrue(r.model.setUnitLatency("B", "150"))
        r.model.save()
        advanceUntilIdle()
        val s = r.model.state.value
        assertEquals("Saved on the hub.", s.actionMessage)
        assertEquals(1, s.saveWarnings.size)
        assertTrue(s.saveWarnings.single(), s.saveWarnings.single().contains("no audio link"))
        assertTrue(r.core.calls.toString(), r.core.calls.any { it.startsWith("alinkSystemSaveReport") })
        r.model.reload()
        advanceUntilIdle()
        assertTrue(r.model.state.value.saveWarnings.isEmpty())
    }

    @Test
    fun p1dcfixACleanSaveHasNoWarnings() = runTest(timeout = 20.seconds) {
        val r = rig(plainUnit = false)
        assertTrue(r.model.setUnitLatency("B", "150"))
        r.model.save()
        advanceUntilIdle()
        val s = r.model.state.value
        assertEquals("Saved on the hub.", s.actionMessage)
        assertTrue(s.saveWarnings.isEmpty())
    }
}
