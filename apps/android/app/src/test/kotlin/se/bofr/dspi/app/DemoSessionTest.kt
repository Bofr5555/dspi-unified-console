package se.bofr.dspi.app

import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.usb.UsbSessionState

class DemoSessionTest {
    @Test
    fun demoBridgeConnectsToTheFakeWithoutUsbAndClosesCleanly() = runBlocking {
        val mainExecutor = Executors.newSingleThreadExecutor()
        val main = mainExecutor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val core = FakeDspiCore.demo()
        val session = startDemoSession(parent, "test", main = main, core = core)
        try {
            withTimeout(5_000) {
                val status = session.bridge.status.first { it.connected }
                assertTrue(status.connected)
                while (session.bridge.selectedSerial != FakeDspiCore.DEMO_SERIAL) delay(10)
            }
            assertNotNull("the bridge listens to the fake while demo runs", core.notifyListener)
            withContext(main) { session.close() }
            // review A2: DspiBridgeImpl.close() ran (it drops the core's listeners)
            assertNull(core.notifyListener)
        } finally {
            parent.cancel()
            main.close()
        }
        assertEquals(FakeDspiCore.DEMO_SERIAL, session.bridge.selectedSerial)
    }

    @Test
    fun aDspiArrivingDuringDemoClosesTheRealDemoStack() = runBlocking {
        val mainExecutor = Executors.newSingleThreadExecutor()
        val main = mainExecutor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val usbState = MutableStateFlow<UsbSessionState>(UsbSessionState.NoDevice)
        val cores = CopyOnWriteArrayList<FakeDspiCore>()
        // Any bridge works as the "device" one; the fake is enough (WP AN38: the real core would work here too,
        // DspiRobolectricTestRunner keeps it out of the sandbox, see DemoCoresInSequenceAn38Test)
        val device = startDemoSession(parent, "device stand-in", main = main, core = FakeDspiCore.demo())
        try {
            val modes = withContext(main) {
                AppModeController(
                    deviceBridge = device.bridge,
                    usbState = usbState,
                    usbDevices = MutableStateFlow(emptyList()),
                    scope = parent,
                    demoFactory = { startDemoSession(parent, "test", main = main, core = FakeDspiCore.demo().also { cores += it }) },
                    forwardUiVisible = {}
                ).also { assertTrue(it.enterDemo()) }
            }
            withTimeout(5_000) { while (cores.single().notifyListener == null) delay(10) }
            usbState.value = UsbSessionState.PermissionRequired("")
            withTimeout(5_000) { while (modes.mode.value != AppMode.DEVICE) delay(10) }
            assertNull(cores.single().notifyListener)
            assertSame(device.bridge, modes.bridge.value)
        } finally {
            withContext(main) { device.close() }
            parent.cancel()
            main.close()
        }
    }
}
