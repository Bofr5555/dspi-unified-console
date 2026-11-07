// WP AN18, "export first": macOS FirmwareUpdateView offers Export
// Configuration... before an update (a format change between versions can
// leave the presets unreadable). Saved from here, through the bridge's
// export, so the user stays on the update. Names and type as PRESETS'
// Export Device Configuration (PresetDocumentFiles).
package se.bofr.dspi.feature.firmware

import se.bofr.dspi.bridge.ConfigFiles
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.bool
import se.bofr.dspi.ui.json.str

object ConfigurationBackup {
    const val FILE_EXTENSION = "dspipreset"

    /** The picker's suggested name. */
    const val FILE_NAME = "DSPi Configuration.$FILE_EXTENSION"

    /** `.dspipreset` has no registered type. */
    const val MIME_TYPE = "application/octet-stream"

    /** The configuration's name inside the document: the file's name without its extensions. */
    fun documentName(displayName: String?): String {
        var name = displayName?.substringAfterLast('/')?.trim().orEmpty()
        while (true) {
            val dot = name.lastIndexOf('.')
            if (dot <= 0 || name.substring(dot + 1).lowercase() !in setOf(FILE_EXTENSION, "json", "bin")) break
            name = name.substring(0, dot)
        }
        return name.trim().ifEmpty { FILE_NAME.removeSuffix(".$FILE_EXTENSION") }
    }

    /**
     * Why [exported] ([ConfigFiles.exportConfiguration]'s reply) is not a
     * document to save: empty with no device, or the core's refusal. Null when it is one.
     */
    fun refusal(exported: String): String? {
        if (exported.isEmpty()) return "No device connected."
        val reply = if (exported.startsWith("{")) Json.parseObject(exported) else null
        if (reply?.containsKey("ok") == true && reply.bool("ok") != true) {
            return reply.str("error")?.ifEmpty { null } ?: "The configuration could not be exported."
        }
        return null
    }

    /** Exports through [files] and hands the document to [save]; returns what to tell the user. */
    suspend fun export(files: ConfigFiles, displayName: String?, save: suspend (String) -> Unit): String {
        val text = files.exportConfiguration(documentName(displayName))
        refusal(text)?.let { return it }
        return try {
            save(text)
            "The configuration was saved. You can bring it back with Import Device Configuration."
        } catch (e: Exception) {
            "The configuration could not be saved: ${e.message ?: e.javaClass.simpleName}."
        }
    }
}
