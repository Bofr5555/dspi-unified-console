package se.bofr.dspi.app

import java.lang.reflect.Proxy
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.usb.DspiUsbDevice
import se.bofr.dspi.usb.UsbIds
import se.bofr.dspi.usb.UsbSessionState

/** WP AN7: a user's device switch runs the Unsaved Changes flow (M42) on the bridge it leaves. */
@OptIn(ExperimentalCoroutinesApi::class)
class DeviceSwitchAn7Test {
    /** A bridge with nothing unsaved; identity is all the controller needs of it. */
    private fun cleanBridge(): DspiBridge = Proxy.newProxyInstance(DspiBridge::class.java.classLoader, arrayOf(DspiBridge::class.java)) {
            p,
            m,
            a
        ->
        when (m.name) {
            "getConnected" -> false
            "hashCode" -> System.identityHashCode(p)
            "equals" -> p === a!![0]
            else -> null
        }
    } as DspiBridge

    private class Demo(override val bridge: DspiBridge) : DemoSession {
        var closed = 0

        override fun close() {
            closed++
        }
    }

    private class Remote(override val bridge: DspiBridge) : RemoteSession {
        override val state = MutableStateFlow<LinkSessionState>(LinkSessionState.Idle)

        override fun connect(hub: LinkHub) {
            state.value = LinkSessionState.Connected(hub, den, "control")
        }

        override fun submitPin(pin: String) {}

        override fun cancelPin() {}

        override fun chooseDevice(device: LinkDevice) {}

        override fun retryNow() {}

        override fun disconnect() {}

        override fun close() {}
    }

    private companion object {
        val den = LinkDevice(0, "0123456789ABCDEF", "Den", "1.1.6", 0, 1, "online", "usb", 0)
    }

    private val hub = LinkHub("6f1a", "Studio Mac", "console", "none", 1, emptyList(), "192.168.1.20", 11915)
    private val usbState = MutableStateFlow<UsbSessionState>(UsbSessionState.Open("x", UsbIds.DSPI_VID))
    private val demos = mutableListOf<Demo>()

    private suspend fun TestScope.connectedBridge(): DspiBridgeImpl {
        val core = FakeDspiCore.seeded()
        val d = StandardTestDispatcher(testScheduler)
        val b = DspiBridgeImpl(core, backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
        check(b.attachDevice(42, core.serial, DspiConstants.VENDOR_ID))
        runCurrent()
        return b
    }

    /** Let queued FFI work and the 30 ms notify batching land. */
    private fun TestScope.settle() {
        runCurrent()
        advanceTimeBy(500)
        runCurrent()
    }

    /** On the test's own dispatcher, as the app's main thread: a bridge's fire-and-forget writes keep their order. */
    private fun TestScope.controller(device: DspiBridge, remote: () -> RemoteSession = { Remote(cleanBridge()) }) = AppModeController(
        deviceBridge = device,
        usbState = usbState,
        usbDevices = MutableStateFlow<List<DspiUsbDevice>>(emptyList()),
        scope = backgroundScope,
        demoFactory = { Demo(cleanBridge()).also { demos += it } },
        forwardUiVisible = {},
        remoteFactory = remote
    ).also { settle() }

    private suspend fun TestScope.dirtyDevice(): DspiBridgeImpl = connectedBridge().also {
        it.setBypass(!it.bypass)
        settle()
        assertTrue(it.presetDirty.value)
    }

    @Test
    fun an7ASwitchWithADirtyPresetAsksFirstAndCancelStays() = runTest {
        val b = dirtyDevice()
        val c = controller(b)
        c.switchTo(DeviceChoice.Demo)
        settle()
        val alert = c.switchAlerts.alert.value!!
        assertEquals("Unsaved Changes", alert.messageText)
        assertEquals(listOf("Save", "Discard", "Cancel"), alert.buttons.map { it.title })
        assertEquals("nothing switched while asking", AppMode.DEVICE, c.mode.value)
        assertTrue(demos.isEmpty())
        c.switchAlerts.dismiss() // Cancel
        settle()
        assertNull(c.switchAlerts.alert.value)
        assertEquals(AppMode.DEVICE, c.mode.value)
        assertSame(b, c.bridge.value)
        assertTrue(demos.isEmpty())
        assertTrue(b.presetDirty.value)
    }

    @Test
    fun an7DiscardSwitchesAndLeavesTheUnitAsItIs() = runTest {
        val b = dirtyDevice()
        val c = controller(b)
        c.switchTo(DeviceChoice.Demo)
        settle()
        val alert = c.switchAlerts.alert.value!!
        c.switchAlerts.respond(alert.button("Discard"))
        settle()
        assertEquals(AppMode.DEMO, c.mode.value)
        assertSame(demos.single().bridge, c.bridge.value)
        assertTrue("Discard reverts nothing on the unit", b.presetDirty.value)
    }

    @Test
    fun an7SaveWritesThePresetThenSwitches() = runTest {
        val b = dirtyDevice()
        val c = controller(b)
        c.switchTo(DeviceChoice.Hub(hub))
        settle()
        val alert = c.switchAlerts.alert.value!!
        c.switchAlerts.respond(alert.button("Save"))
        settle()
        assertFalse("saved", b.presetDirty.value)
        assertNull(c.switchAlerts.alert.value)
        assertEquals(AppMode.REMOTE, c.mode.value)
    }

    @Test
    fun an7ACleanSwitchDoesNotAsk() = runTest {
        val b = connectedBridge()
        settle()
        assertFalse(b.presetDirty.value)
        val c = controller(b)
        c.switchTo(DeviceChoice.Demo)
        settle()
        assertNull(c.switchAlerts.alert.value)
        assertEquals(AppMode.DEMO, c.mode.value)
        // the current choice is no switch: nothing happens
        c.switchTo(DeviceChoice.Demo)
        assertEquals(1, demos.size)
    }

    @Test
    fun an7LeavingADirtyHubForUsbAsksToo() = runTest {
        val remoteBridge = dirtyDevice()
        val c = controller(cleanBridge()) { Remote(remoteBridge) }
        c.switchTo(DeviceChoice.Hub(hub))
        settle()
        assertEquals(AppMode.REMOTE, c.mode.value)
        // D9's "Use USB" is a user's switch as well
        c.answerUsbPrompt(useUsb = true)
        settle()
        assertEquals("Unsaved Changes", c.switchAlerts.alert.value!!.messageText)
        assertEquals(AppMode.REMOTE, c.mode.value)
        c.switchAlerts.respond(c.switchAlerts.alert.value!!.button("Discard"))
        settle()
        assertEquals(AppMode.DEVICE, c.mode.value)
    }

    @Test
    fun an7DspiLinkEntryIsADialogNotASwitch() = runTest {
        val b = dirtyDevice()
        val c = controller(b)
        c.switchTo(DeviceChoice.Network) // a dialog, not a switch
        assertNull(c.switchAlerts.alert.value)
        assertEquals(AppMode.DEVICE, c.mode.value)
    }
}
