package se.bofr.dspi.feature.presets

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.ui.alert.AlertHost

/**
 * The Kotlin fake with P1DF's preview and per-block import and P1SA's guarded
 * import, as far as the flow needs them (the real core's are tested in :app's
 * SnapshotsWpP1an1Test).
 */
class BlockImportFake(private val guardDefault: Boolean = false) : FakeDspiCore(presetFake().getState()) {
    var guarded: Long? = null

    override fun configPreview(text: String, options: Long, eqChannels: Long): String {
        calls += "configPreview($options)"
        return """{"ok":true,"count":3,"rows":[
            {"block":"outputs","section":"Out 1","item":"Gain","before":0,"after":-3.5},
            {"block":"eq","section":"Input L","item":"Band 1 gain","before":"0 dB","after":"-2 dB"},
            {"block":"volumes","section":"Master","item":"Volume","before":"-20 dB","after":"-10 dB"}],
            "summary":["Out 1 gain: 0 dB → -3.5 dB"],"notes":["Channel 9 is not on this device."]}"""
    }

    override fun configImportSelected(text: String, options: Long, eqChannels: Long): String {
        calls += "configImportSelected($options)"
        return configImport(text, options and 3L)
    }

    override fun applyGuardDefault() = guardDefault

    override fun applyGuardedImport(text: String, options: Long, eqChannels: Long, timeoutS: Long): GuardedApply {
        calls += "applyGuardedImport($options)"
        guarded = options
        return GuardedApply(1, GuardPhase.WAITING, "Room", guardDefault, 30_000, 30_000, "", "", emptyList(), emptyList(), "")
    }

    override fun applyStatus(): GuardedApply = if (guarded ==
        null
    ) {
        GuardedApply(0, GuardPhase.IDLE, "", false, 0, 0, "", "", emptyList(), emptyList(), "")
    } else {
        applyGuardedImport("", guarded!!, 0, 0)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class ImportPreviewWpP1an1Test {
    private suspend fun kotlinx.coroutines.test.TestScope.opened(core: BlockImportFake): Pair<FileMenuActions, AlertHost> {
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        val doc = f.configurationDocument("Room.dspipreset")!!
        f.importConfiguration(doc)
        runCurrent()
        return f to host
    }

    @Test
    fun p1an1PreviewShowsTheRowsAndTicksTheChangedBlocksButVolumesAndHardware() = runTest {
        val (f, host) = opened(BlockImportFake())
        val p = f.preview.value!!
        assertNull(host.alert.value)
        assertTrue(p.diff.ok)
        assertEquals(3, p.diff.rows.size)
        assertEquals("-3.5", p.diff.rows.first().after)
        assertEquals(setOf(ImportBlock.OUTPUTS, ImportBlock.EQ), p.selected)
        assertEquals(listOf("Channel 9 is not on this device."), p.diff.notes)
        assertFalse("USB: rollback is the user's choice, off at first", p.guarded)
        assertFalse(p.intro.contains("always applied"))
        f.setBlock(ImportBlock.EQ, false)
        assertEquals(listOf("outputs"), f.preview.value!!.chosenRows.map { it.block })
        f.setBlock(ImportBlock.VOLUMES, true)
        assertEquals(listOf("outputs", "volumes"), f.preview.value!!.chosenRows.map { it.block })
        f.cancelPreview()
        assertNull(f.preview.value)
    }

    @Test
    fun p1an1ImportAppliesOnlyTheChosenBlocksWithTheToken() = runTest {
        val core = BlockImportFake()
        val (f, host) = opened(core)
        f.setBlock(ImportBlock.EQ, false)
        f.applyPreview()
        runCurrent()
        val bits = DspiConstants.IMPORT_OUTPUTS.toLong() or DspiConstants.IMPORT_BY_BLOCK.toLong()
        assertTrue(core.calls.toString(), core.calls.contains("configImportSelected($bits)"))
        assertNull(f.preview.value)
        assertNull(host.progress.value)
        // -- WP P1DCFIX -- (F5) a clean import is the page's notice line, no success modal
        val noticed = f.notice.value.isNotEmpty() && host.alert.value == null
        assertTrue(noticed || host.alert.value?.messageText == "Import Device Configuration")
    }

    @Test
    fun p1an1RollbackImportsGuardedWithoutAToken() = runTest {
        val core = BlockImportFake()
        val (f, host) = opened(core)
        f.setGuarded(true)
        f.applyPreview()
        runCurrent()
        assertTrue(core.calls.toString(), core.calls.any { it.startsWith("applyGuardedImport(") })
        assertFalse(core.calls.any { it.startsWith("configImportSelected(") })
        assertNull("the bar says it, not an alert", host.alert.value)
    }

    @Test
    fun p1an1OverDspiLinkRollbackIsOnByDefault() = runTest {
        val (f, _) = opened(BlockImportFake(guardDefault = true))
        val p = f.preview.value!!
        assertTrue(p.guarded)
        assertTrue(p.guardDefault)
    }

    @Test
    fun p1an1NothingChosenImportsNothing() = runTest {
        val core = BlockImportFake()
        val (f, _) = opened(core)
        ImportBlock.entries.forEach { f.setBlock(it, false) }
        f.applyPreview()
        runCurrent()
        assertFalse(core.calls.any { it.startsWith("configImportSelected(") || it.startsWith("applyGuardedImport(") })
        assertTrue("the preview stays open", f.preview.value != null)
    }

    @Test
    fun p1an1APreviewTheCoreCannotListStillOffersTheBlocks() = runTest {
        // The plain fake has no preview: the sheet says why and offers every block but volumes and hardware
        val b = attached(presetFake())
        val f = FileMenuActions(b, this, AlertHost())
        f.importConfiguration(f.configurationDocument("Room.dspipreset")!!)
        runCurrent()
        val p = f.preview.value!!
        assertFalse(p.diff.ok)
        assertTrue(p.diff.error, p.diff.error.startsWith("The changes could not be listed"))
        assertEquals(ImportBlock.entries.toSet() - ImportBlock.VOLUMES - ImportBlock.HARDWARE, p.selected)
    }

    @Test
    fun p1an1DiffJsonAndTemplateNames() {
        val bad = DocDiff.parse("""{"ok":false,"error":"not a document"}""")
        assertFalse(bad.ok)
        assertEquals("not a document", bad.error)
        assertFalse(DocDiff.parse("nonsense").ok)
        assertEquals("My_room.dspipreset", OfflineFiles.templateFileName("My/room"))
        assertEquals("Template.dspipreset", OfflineFiles.templateFileName("  ..  "))
        val t = OfflineFiles.parseTemplates(
            """[{"id":"builtin:K340","name":"K340","builtin":true,"platform":"RP2350","status":"awaiting approval"},{"name":"no id"}]"""
        )
        assertEquals(listOf(TemplateEntry("builtin:K340", "K340", true, "RP2350", "awaiting approval")), t)
    }
}
