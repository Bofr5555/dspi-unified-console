package se.bofr.dspi.feature.presets

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertResponse

@OptIn(ExperimentalCoroutinesApi::class)
class FileMenuActionsTest {
    @Test
    fun configurationRoundTrip() = runTest {
        val core = BlockImportFake() // -- WP P1AN1 -- with the per-block import
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        assertTrue(f.canExportConfiguration())
        val text = f.configurationDocument("Living Room.dspipreset")!!
        assertTrue(text.contains("\"Living Room\""))
        f.configurationExported(text)
        assertEquals("Success", host.alert.value!!.messageText)
        assertTrue(host.alert.value!!.informativeText.startsWith("Configuration exported.\n\n"))
        host.press("OK")

        // -- WP P1AN1 -- the import opens the preview (blocks, rollback) instead of the two-checkbox alert
        f.importConfiguration(text)
        runCurrent()
        assertNull(host.alert.value)
        assertTrue(f.preview.value!!.selected.isNotEmpty())
        f.setBlock(ImportBlock.VOLUMES, true)
        f.applyPreview()
        runCurrent()
        assertNull(host.progress.value)
        assertTrue(core.presetCalls().any { it.startsWith("configImport") })
        assertTrue(host.alert.value!!.messageText in setOf("Success", "Import Device Configuration"))
    }

    @Test
    fun importingSomethingElseIsAnError() = runTest {
        val b = attached()
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        f.importConfiguration("not a configuration")
        assertEquals("Error", host.alert.value!!.messageText)
    }

    @Test
    fun exportNeedsADevice() = runTest {
        val b = detached()
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        assertFalse(f.canExportConfiguration())
        assertEquals("No device connected.", host.alert.value!!.informativeText)
    }

    /** WP CG18: a refusal is an alert, not a file and not a crash. */
    @Test
    fun aRefusedExportIsReportedAndGivesNoDocument() = runTest {
        val core = presetFake()
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        core.failNext("configExport") { DspiException.Unsupported() }
        assertNull(f.configurationDocument("x.dspipreset"))
        assertEquals("Error", host.alert.value!!.messageText)
        assertTrue(host.alert.value!!.informativeText.contains("not supported by the connected firmware"))
        host.press("OK")
        core.failNext("filtersExport") { DspiException.Unsupported() }
        assertNull(f.filtersDocument())
        assertTrue(host.alert.value!!.informativeText.contains("not supported by the connected firmware"))
    }

    @Test
    fun filterRoundTrip() = runTest {
        val core = presetFake()
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        val text = f.filtersDocument()!!
        f.filtersExported()
        assertEquals("Filters exported successfully", host.alert.value!!.informativeText)
        host.press("OK")
        f.importFilters(text)
        runCurrent()
        val pick = host.alert.value!!
        assertEquals("Import Filters", pick.messageText)
        assertTrue(pick.checkboxes.isNotEmpty())
        host.press("Import", AlertResponse(checked = pick.checkboxes.filter { it.checked }.map { it.key }.toSet()))
        runCurrent()
        assertTrue(core.presetCalls().any { it.startsWith("filtersImport") })
        assertEquals("Success", host.alert.value!!.messageText)
    }

    @Test
    fun rewFileGetsTheSingleChannelPrompt() = runTest {
        val b = attached()
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        f.importFilters("Preamp: -6.0 dB\nFilter 1: ON PK Fc 100 Hz Gain -3.0 dB Q 1.00\nFilter 2: ON PK Fc 1000 Hz Gain 2.0 dB Q 1.00\n")
        runCurrent()
        val pick = host.alert.value!!
        assertTrue(pick.informativeText, pick.informativeText.startsWith("Found 2 filter(s)"))
        assertTrue(pick.informativeText.endsWith("Select which channel(s) to apply them to:"))
        // Inputs 1–2 ticked by default
        assertEquals(setOf("0", "1"), pick.checkboxes.filter { it.checked }.map { it.key }.toSet())
    }
}
