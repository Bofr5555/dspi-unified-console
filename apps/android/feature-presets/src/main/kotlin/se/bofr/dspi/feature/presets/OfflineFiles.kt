// WP P1AN1: offline files (PRO1 W7, P1OF) on Android. Open File… and the
// templates open a document as a device of its own, with nothing on USB: the
// app leaves the current device (after the Unsaved Changes flow) for a session
// on the document, and the strip says "File: <name>". Save File… writes the
// document back; Save as Template… files the open device or document as a
// template in the app's own folder, where the template list finds it.
package se.bofr.dspi.feature.presets

import android.content.Context
import androidx.compose.runtime.staticCompositionLocalOf
import java.io.File
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.bool
import se.bofr.dspi.ui.json.str

/** Opens a document offline: the app's (AppModeController); the core's `open_document` does the rest. */
fun interface DocumentOpener {
    fun open(text: String, fileName: String)
}

/** The app's [DocumentOpener]; null where there is none (previews, tests): Open File… is then off. */
val LocalDocumentOpener = staticCompositionLocalOf<DocumentOpener?> { null }

/** One template of `template_list`. */
data class TemplateEntry(val id: String, val name: String, val builtin: Boolean, val platform: String, val status: String)

object OfflineFiles {
    /** The user's templates: the app's own files. */
    fun templatesFolder(context: Context): File = File(context.filesDir, "templates")

    fun parseTemplates(json: String): List<TemplateEntry> =
        (Json.parse(json) as? List<*>).orEmpty().filterIsInstance<Map<String, Any?>>().map { m ->
            TemplateEntry(
                id = m.str("id").orEmpty(),
                name = m.str("name").orEmpty(),
                builtin = m.bool("builtin") == true,
                platform = m.str("platform").orEmpty(),
                status = m.str("status").orEmpty()
            )
        }.filter { it.id.isNotEmpty() }

    /** A file name for template [name]: what a file system takes, the document's extension added. */
    fun templateFileName(name: String): String {
        val base = name.trim().replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "_").trim('.', ' ').take(80).ifEmpty { "Template" }
        return "$base.${PresetDocumentFiles.FILE_EXTENSION}"
    }
}
