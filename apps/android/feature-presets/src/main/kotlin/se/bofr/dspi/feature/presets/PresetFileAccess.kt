// WP P1AN1: the SAF pickers and file reads/writes of PresetsViewModel, for the
// Snapshots screen and the offline files (Open File…, Save File…), with the
// failures reported on the caller's alert host. Not JVM-tested (SAF).
package se.bofr.dspi.feature.presets

import android.content.ActivityNotFoundException
import android.content.Context
import android.net.Uri
import kotlinx.coroutines.Deferred
import se.bofr.dspi.ui.alert.AlertHost

class PresetFileAccess(private val app: Context) {
    /** The open picker for any document (a `.dspipreset` has no registered type). */
    fun startOpen(context: Context): Deferred<Uri?> = DocumentPicker.startOpen(context, arrayOf("*/*"))

    /** The save picker, [suggestedName] in its name field. */
    fun startCreate(context: Context, suggestedName: String): Deferred<Uri?> =
        DocumentPicker.startCreate(context, suggestedName, PresetDocumentFiles.CONFIGURATION_MIME)

    suspend fun await(picked: Deferred<Uri?>, host: AlertHost): Uri? = try {
        picked.await()
    } catch (e: ActivityNotFoundException) {
        host.showError("No app is available to pick a file.")
        null
    }

    suspend fun read(uri: Uri, host: AlertHost): String? = try {
        PresetFiles.readText(app, uri)
    } catch (e: Exception) {
        host.showError("Failed to read file: ${e.message ?: e.javaClass.simpleName}")
        null
    }

    suspend fun write(uri: Uri, text: String, host: AlertHost): Boolean = try {
        PresetFiles.writeText(app, uri, text)
        true
    } catch (e: Exception) {
        host.showError("Failed to write file: ${e.message ?: e.javaClass.simpleName}")
        false
    }

    suspend fun displayName(uri: Uri): String? = PresetFiles.displayName(app, uri)
}
