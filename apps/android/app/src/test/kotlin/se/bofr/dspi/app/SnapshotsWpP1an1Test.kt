package se.bofr.dspi.app

import java.io.File
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineDispatcher
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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.AbSide
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.feature.presets.FileMenuActions
import se.bofr.dspi.feature.presets.ImportBlock
import se.bofr.dspi.feature.presets.snapshots.SnapshotsModel
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.usb.UsbSessionState

/**
 * WP P1AN1: the Snapshots screen's model, the import preview and the offline
 * files on the real core's demo device (AN28's stack, outside Robolectric as
 * AN38's tests): capture and recall, A/B, a guarded recall with Revert and with
 * the timeout, A/B refused while a guard waits and ended by its revert, the
 * preview's rows and blocks, and a document opened offline, edited and saved.
 * The snapshot store is under the module's build folder (never a tmpfs).
 */
class SnapshotsWpP1an1Test {
    private class Stack(val bridge: DspiBridge, val main: CoroutineDispatcher, val scope: CoroutineScope, val folder: File)

    private fun withDemo(name: String, block: suspend Stack.() -> Unit) = runBlocking {
        val executor = Executors.newSingleThreadExecutor()
        val main = executor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val folder = File("build/wp-p1an1/$name").apply {
            deleteRecursively()
            mkdirs()
        }
        val session = startDemoSession(parent, "p1an1", main = main)
        try {
            withTimeout(15_000) {
                session.bridge.status.first { it.connected }
                session.bridge.writesAllowed.first { it }
            }
            Stack(session.bridge, main, parent, folder).block()
        } finally {
            withContext(main) { session.close() }
            parent.cancel()
            executor.shutdown()
        }
    }

    private suspend fun until(what: String, ms: Long = 15_000, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + ms
        while (!cond()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            delay(20)
        }
    }

    private suspend fun Stack.model(): SnapshotsModel = withContext(main) {
        SnapshotsModel(bridge, scope, folder.path).also { it.load() }
    }.also { m -> until("loaded") { !m.busy.value } }

    private suspend fun Stack.setGain(out: Int, db: Float) {
        withContext(main) { bridge.setOutputGain(out, db) }
        until("gain $db") { bridge.outputGainDB(out) == db }
    }

    @Test
    fun p1an1CaptureThenRecallBringsTheValueBack() = withDemo("recall") {
        val m = model()
        setGain(0, -4f)
        withContext(main) { m.capture("Before", "a note") }
        until("captured") { !m.busy.value && m.items.value.size == 1 }
        assertEquals("a note", m.items.value.single().note)
        setGain(0, -11f)
        withContext(main) { m.recall(m.items.value.single()) }
        until("recalled") { !m.busy.value && bridge.outputGainDB(0) == -4f }
        assertTrue(m.result.value, m.result.value.startsWith("Recalled \"Before\""))
        assertEquals("a plain recall on USB has no guard", GuardPhase.IDLE, bridge.snapshots.guard.value.phase)
    }

    @Test
    fun p1an1AbTogglesBetweenTwoSnapshotsAndKeepEndsIt() = withDemo("ab") {
        val m = model()
        setGain(0, -2f)
        withContext(main) { m.capture("A side", "") }
        until("A") { !m.busy.value && m.items.value.size == 1 }
        setGain(0, -9f)
        withContext(main) { m.capture("B side", "") }
        until("B") { !m.busy.value && m.items.value.size == 2 }
        val a = m.items.value.first { it.name == "A side" }
        val b = m.items.value.first { it.name == "B side" }
        withContext(main) {
            m.chooseA(a.id)
            m.chooseB(b.id)
            m.startCompare()
        }
        until("on A") { !m.busy.value && bridge.snapshots.compare.value.active && bridge.outputGainDB(0) == -2f }
        assertEquals(AbSide.A, bridge.snapshots.compare.value.side)
        withContext(main) { m.toggle() }
        until("on B") { !m.busy.value && bridge.snapshots.compare.value.side == AbSide.B && bridge.outputGainDB(0) == -9f }
        withContext(main) { m.showDiff() }
        until("diff") { !m.busy.value && m.diff.value != null }
        val diff = m.diff.value!!
        assertTrue(diff.error, diff.ok)
        assertTrue("the diff names the output gain: ${diff.rows}", diff.rows.any { it.block == ImportBlock.OUTPUTS.key })
        withContext(main) { m.keep(AbSide.B) }
        until("kept") { !m.busy.value && !bridge.snapshots.compare.value.active }
        assertEquals(-9f, bridge.outputGainDB(0))
    }

    @Test
    fun p1an1GuardedRecallWaitsBlocksAbAndRevertEndsTheCompare() = withDemo("guard") {
        val m = model()
        assertFalse("USB: the guard is the user's choice", m.guardDefault.value)
        setGain(1, -3f)
        withContext(main) { m.capture("One", "") }
        until("one") { !m.busy.value && m.items.value.size == 1 }
        setGain(1, -15f)
        withContext(main) { m.capture("Two", "") }
        until("two") { !m.busy.value && m.items.value.size == 2 }
        val one = m.items.value.first { it.name == "One" }
        val two = m.items.value.first { it.name == "Two" }
        withContext(main) {
            m.chooseA("")
            m.chooseB(one.id)
            m.startCompare()
        }
        until("compare") { !m.busy.value && bridge.snapshots.compare.value.active }
        // The device is on A (-15 dB, captured now); recall One with rollback
        withContext(main) {
            m.setSafeRecall(true)
            m.recall(one)
        }
        until("waiting") { !m.busy.value && bridge.snapshots.guard.value.phase == GuardPhase.WAITING }
        assertEquals(-3f, bridge.outputGainDB(1))
        assertTrue(bridge.snapshots.guardWaiting)
        // A/B is off while the guard waits: the model says so and sends nothing
        withContext(main) { m.toggle() }
        until("refused") { !m.busy.value }
        assertEquals(SnapshotsModel.FINISH_SAFE_APPLY, m.result.value)
        assertEquals(-3f, bridge.outputGainDB(1))
        // Revert: the value from before comes back and the compare has ended
        val r = withContext(main) { bridge.snapshots.revert() }
        assertEquals(GuardPhase.REVERTED, r.getOrThrow().phase)
        until("reverted") { bridge.outputGainDB(1) == -15f }
        until("compare ended") { !bridge.snapshots.compare.value.active }
        assertTrue(two.id.isNotEmpty())
    }

    @Test
    fun p1an1AGuardNotAnsweredRevertsOnThePoll() = withDemo("timeout") {
        val m = model()
        setGain(2, -1f)
        withContext(main) { m.capture("Keep me", "") }
        until("captured") { !m.busy.value && m.items.value.size == 1 }
        setGain(2, -20f)
        val id = m.items.value.single().id
        val g = withContext(main) { bridge.snapshots.recall(id, guarded = true, timeoutS = 1) }.getOrThrow().guard
        assertEquals(GuardPhase.WAITING, g?.phase)
        assertEquals(-1f, bridge.outputGainDB(2))
        // Nobody answers: the poll (the bar's, while visible) reverts at the timeout
        val end = System.currentTimeMillis() + 10_000
        while (bridge.snapshots.guard.value.phase == GuardPhase.WAITING && System.currentTimeMillis() < end) {
            withContext(main) { bridge.snapshots.poll() }
            delay(100)
        }
        assertEquals(GuardPhase.REVERTED, bridge.snapshots.guard.value.phase)
        assertEquals("timeout", bridge.snapshots.guard.value.reason)
        until("back") { bridge.outputGainDB(2) == -20f }
    }

    @Test
    fun p1an1ImportPreviewListsTheChangesAndAppliesTheChosenBlocks() = withDemo("import") {
        val host = AlertHost()
        val files = withContext(main) { FileMenuActions(bridge, scope, host) }
        setGain(0, -5f)
        val doc = withContext(main) { files.configurationDocument("Room.dspipreset") }!!
        setGain(0, -12f)
        withContext(main) { files.importConfiguration(doc) }
        until("preview") { files.preview.value != null }
        val p = files.preview.value!!
        assertTrue(p.diff.error, p.diff.ok)
        assertTrue("the gain is a row: ${p.diff.rows}", p.diff.rows.any { it.block == ImportBlock.OUTPUTS.key })
        assertTrue(ImportBlock.OUTPUTS in p.selected)
        assertFalse("volumes stay out unless asked", ImportBlock.VOLUMES in p.selected)
        assertFalse("USB: no rollback by default", p.guarded)
        // Without the outputs block the gain is not among the chosen rows
        withContext(main) { files.setBlock(ImportBlock.OUTPUTS, false) }
        assertTrue(files.preview.value!!.chosenRows.none { it.block == ImportBlock.OUTPUTS.key })
        withContext(main) {
            files.setBlock(ImportBlock.OUTPUTS, true)
            files.setGuarded(true)
            files.applyPreview()
        }
        until("guard") { bridge.snapshots.guard.value.phase == GuardPhase.WAITING }
        assertNull(files.preview.value)
        until("applied") { bridge.outputGainDB(0) == -5f }
        withContext(main) { bridge.snapshots.keep() }.getOrThrow()
        assertEquals(GuardPhase.KEPT, bridge.snapshots.guard.value.phase)
        assertEquals(-5f, bridge.outputGainDB(0))
        // And unguarded, with the outputs block left out: the gain stays
        setGain(0, -7f)
        withContext(main) { files.importConfiguration(doc) }
        until("preview 2") { files.preview.value != null }
        withContext(main) {
            files.setBlock(ImportBlock.OUTPUTS, false)
            files.setBlock(ImportBlock.EQ, true)
            files.applyPreview()
        }
        // -- WP P1DCFIX -- (F5) a clean import is the page's notice line; only a report with problems is an alert
        until("imported") { files.notice.value.isNotEmpty() || host.alert.value != null }
        assertTrue("no success modal", host.alert.value?.messageText != "Success")
        assertEquals(-7f, bridge.outputGainDB(0))
    }

    @Test
    fun p1an1ADocumentOpensOfflineEditsAndSavesBack() = runBlocking {
        val executor = Executors.newSingleThreadExecutor()
        val main = executor.asCoroutineDispatcher()
        val parent = CoroutineScope(SupervisorJob() + main)
        val device = startDemoSession(parent, "device stand-in", main = main)
        try {
            withTimeout(15_000) { device.bridge.status.first { it.connected } }
            withContext(main) { device.bridge.setOutputGain(0, -6f) }
            until("gain") { device.bridge.outputGainDB(0) == -6f }
            val host = AlertHost()
            val doc = withContext(main) { FileMenuActions(device.bridge, parent, host).configurationDocument("Room.dspipreset") }!!
            val modes =
                withContext(main) {
                    AppModeController(
                        deviceBridge = device.bridge,
                        usbState = MutableStateFlow<UsbSessionState>(UsbSessionState.NoDevice),
                        usbDevices = MutableStateFlow(emptyList()),
                        scope = parent,
                        demoFactory = { error("not here") },
                        forwardUiVisible = {},
                        documentFactory = { text, name -> startDocumentSession(parent, "p1an1", text, name, main = main) }
                    )
                }
            withContext(main) { modes.openDocument(doc, "Room.dspipreset") }
            // The device has unsaved changes: the switch asks first, as any device switch
            until("unsaved changes asked") { modes.switchAlerts.alert.value != null }
            withContext(main) { modes.switchAlerts.respond(modes.switchAlerts.alert.value!!.button("Discard")) }
            until("document mode") { modes.mode.value == AppMode.DEMO }
            val b = modes.bridge.value
            withTimeout(15_000) { b.status.first { it.connected } }
            until("offline") { b.snapshots.offline.value != null }
            assertEquals("Room.dspipreset", b.snapshots.offline.value!!.fileName)
            assertEquals(
                ConnectionStrip.Document("Room.dspipreset"),
                connectionStrip(AppMode.DEMO, UsbSessionState.NoDevice, document = "Room.dspipreset")
            )
            until("document's value") { b.outputGainDB(0) == -6f }
            withContext(main) { b.setOutputGain(0, -13.5f) }
            until("edited") { b.outputGainDB(0) == -13.5f }
            val saved = withContext(main) { b.snapshots.saveDocument() }.getOrThrow()
            assertTrue("the saved document holds the edit", saved.contains("-13.5"))
            val template = withContext(main) { b.snapshots.saveAsTemplate("My room") }.getOrThrow()
            assertTrue(template.contains("\"template\""))
            assertTrue("the shipped K340 template is listed", b.snapshots.templates("").contains("builtin"))
            withContext(main) { modes.exitDemo() }
            assertEquals(AppMode.DEVICE, modes.mode.value)
            assertNotNull(device.bridge)
        } finally {
            withContext(main) { device.close() }
            parent.cancel()
            executor.shutdown()
        }
    }
}
