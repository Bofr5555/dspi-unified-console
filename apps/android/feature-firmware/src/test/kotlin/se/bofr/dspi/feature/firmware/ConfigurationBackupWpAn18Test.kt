package se.bofr.dspi.feature.firmware

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.ConfigFiles
import se.bofr.dspi.bridge.ConfirmationToken

/** WP AN18, "export first": Export Configuration... from the update screen. */
class ConfigurationBackupWpAn18Test {
    private class Files(private val reply: String) : ConfigFiles {
        var exportedName: String? = null

        override suspend fun exportConfiguration(name: String): String {
            exportedName = name
            return reply
        }

        override fun inspectConfiguration(text: String) = ""

        override suspend fun importConfiguration(token: ConfirmationToken, text: String, volumes: Boolean, hardware: Boolean) = ""

        override suspend fun exportFilters() = ""

        override suspend fun inspectFilters(text: String) = ""

        override suspend fun importFilters(text: String, wires: List<Int>) = ""
    }

    @Test fun wpAn18TheDocumentIsSavedUnderTheFilesName() = runTest {
        val files = Files("""{"format":"dspipreset"}""")
        var saved: String? = null
        val message = ConfigurationBackup.export(files, "Living room.dspipreset.json") { saved = it }
        assertEquals("Living room", files.exportedName)
        assertEquals("""{"format":"dspipreset"}""", saved)
        assertTrue(message.startsWith("The configuration was saved."))
    }

    @Test fun wpAn18ARefusalIsNotSaved() = runTest {
        var saved = false
        val message = ConfigurationBackup.export(Files("""{"ok":false,"error":"This device's settings could not be exported."}"""), null) {
            saved = true
        }
        assertEquals("This device's settings could not be exported.", message)
        assertEquals(false, saved)
        assertEquals("No device connected.", ConfigurationBackup.refusal(""))
        assertNull(ConfigurationBackup.refusal("""{"ok":true,"name":"x"}"""))
    }

    @Test fun wpAn18AFailedWriteIsReported() = runTest {
        val message = ConfigurationBackup.export(Files("{}"), "a.dspipreset") { throw java.io.IOException("disk full") }
        assertEquals("The configuration could not be saved: disk full.", message)
        assertEquals("DSPi Configuration", ConfigurationBackup.documentName(""))
    }
}
