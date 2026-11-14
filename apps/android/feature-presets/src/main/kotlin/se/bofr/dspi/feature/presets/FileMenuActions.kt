// Port of DSPi-Console (macOS) `DSPi Console/DSPi_ConsoleApp.swift` @ v1.1.6-beta5
// `FileMenuActions` (Import/Export Filters…, Import/Export Device
// Configuration…) with `PresetDocument*.swift` / `FilterFile.swift` UX. The
// documents themselves are the core's (presetfile.rs, filterfile.rs) through
// `DspiBridge.configFiles`, as Linux `qt/src/ConfigFiles.cpp` + `main.qml`
// fileAction do. This class is text-level (JVM-tested); the SAF pickers and
// the share sheet live in PresetFiles.kt.
package se.bofr.dspi.feature.presets

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import se.bofr.dspi.bridge.DangerousOp
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.ui.alert.AlertButton
import se.bofr.dspi.ui.alert.AlertButtonRole
import se.bofr.dspi.ui.alert.AlertCheckbox
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertStyle
import se.bofr.dspi.ui.alert.AlertTextField
import se.bofr.dspi.ui.alert.ConsoleAlert
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.bool
import se.bofr.dspi.ui.json.double
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.str

class FileMenuActions(private val bridge: DspiBridge, private val scope: CoroutineScope, private val host: AlertHost) {
    // ═══ Device configuration ═════════════════════════════════════════════

    /** `exportConfiguration`'s preconditions; false (after an alert) when the export can't start. */
    fun canExportConfiguration(): Boolean {
        if (!bridge.connected) {
            host.showError("No device connected.")
            return false
        }
        // The document's channel ids depend on the platform: never write one
        // before the device has identified itself.
        if (bridge.platformName.isEmpty()) {
            host.showError("Still identifying the device. Try again in a moment.")
            return false
        }
        return true
    }

    /** The `.dspipreset` text named after the chosen file ([displayName]), null (after an alert) on failure. */
    suspend fun configurationDocument(displayName: String?): String? =
        exported(bridge.configFiles.exportConfiguration(PresetDocumentFiles.configurationName(displayName)))

    /** An export's text, or null after an alert when it isn't a file to write ([ConfigFiles.exportConfiguration]). */
    private fun exported(text: String): String? {
        val reply = if (text.startsWith("{")) Json.parseObject(text) else null
        val refused = reply?.containsKey("ok") == true && reply.bool("ok") != true
        if (text.isEmpty() || refused) {
            host.showError(reply?.str("error")?.ifEmpty { null } ?: "No device connected.")
            return null
        }
        return text
    }

    /** The file was written: what went into it. */
    fun configurationExported(documentText: String) = host.showSuccess(PresetDocumentFiles.exportSummary(documentText))

    fun canImportConfiguration(): Boolean {
        if (!bridge.connected) {
            host.showError("No device connected.")
            return false
        }
        return true
    }

    // -- WP P1AN1 -- the import preview (PRO1 W5, W8, W6): what would change, block by block, then apply, with a way back
    private val _preview = MutableStateFlow<ImportPreview?>(null)

    /** The import preview while it is open; the screen draws it ([ImportPreviewDialog]). */
    val preview: StateFlow<ImportPreview?> = _preview.asStateFlow()

    /**
     * A configuration file was picked: inspect it, then open the preview: the
     * core's diff of the file against the device, the blocks to bring in, and
     * "Apply with rollback" (on by default over DSPi Link, OD-P1g).
     */
    fun importConfiguration(text: String) {
        val info = Json.parseObject(bridge.configFiles.inspectConfiguration(text))
        if (info?.bool("ok") != true) {
            host.showError(info?.str("error")?.ifEmpty { null } ?: "The file could not be read.")
            return
        }
        val intro =
            PresetDocumentFiles.importOptionsText(
                platform = info.str("platform").orEmpty(),
                firmware = info.str("firmware").orEmpty(),
                savedUtc = info.str("savedUtc").orEmpty(),
                connectedPlatform = bridge.platformName
            ).substringBefore("\n\nEQ, crossover").trim()
        scope.launch {
            val diff = DocDiff.parse(bridge.snapshots.importPreview(text, ImportBlock.ALL_BITS))
            val guardDefault = bridge.snapshots.guardDefault()
            _preview.value =
                ImportPreview(
                    text = text,
                    name = info.str("name").orEmpty(),
                    intro = intro,
                    diff = diff,
                    selected = ImportBlock.defaults(diff),
                    guarded = guardDefault,
                    guardDefault = guardDefault
                )
        }
    }

    fun setBlock(block: ImportBlock, on: Boolean) {
        _preview.value = _preview.value?.let { p -> p.copy(selected = if (on) p.selected + block else p.selected - block) }
    }

    fun setGuarded(on: Boolean) {
        _preview.value = _preview.value?.copy(guarded = on)
    }

    fun cancelPreview() {
        _preview.value = null
    }

    /**
     * Import the chosen blocks. With rollback: a guarded apply (no token; the
     * countdown bar asks Keep or Revert). Without: GATED CONFIG_IMPORT, the
     * token minted here, the Import button's handler; the report as before.
     */
    fun applyPreview() {
        val p = _preview.value ?: return
        if (p.selected.isEmpty()) return
        _preview.value = null
        if (!bridge.connected) {
            host.showError("No device connected.")
            return
        }
        val options = ImportBlock.bits(p.selected)
        if (p.guarded) {
            scope.launchDeviceSequence {
                host.beginProgress("Importing configuration…")
                val r =
                    try {
                        bridge.snapshots.importGuarded(p.text, options)
                    } finally {
                        host.endProgress()
                    }
                r.exceptionOrNull()?.let { host.showError(it.message ?: "The configuration was not applied.") }
            }
            return
        }
        val token = bridge.dangerousOps.confirm(DangerousOp.CONFIG_IMPORT)
        scope.launchDeviceSequence {
            host.beginProgress("Importing configuration…")
            val result =
                try {
                    Json.parseObject(bridge.snapshots.importSelected(token, p.text, options))
                } finally {
                    host.endProgress()
                }
            report(result)
        }
    }

    private fun report(result: Map<String, Any?>?) {
        if (result?.bool("ok") != true) {
            host.showError(result?.str("error")?.ifEmpty { null } ?: "The configuration was not applied.")
            return
        }
        val message = result.list("lines").orEmpty().filterIsInstance<String>().joinToString("\n\n")
        // Anything the device refused or couldn't do isn't a success: no "Success" heading over it.
        if (result.bool("clean") != false) {
            host.showSuccess(message)
        } else {
            host.show(
                ConsoleAlert(
                    "Import Device Configuration",
                    message,
                    AlertStyle.INFORMATIONAL,
                    listOf(AlertButton("OK", AlertButtonRole.DEFAULT))
                )
            )
        }
    }

    // ═══ Offline files (-- WP P1AN1 --, P1OF) ════════════════════════════

    /** A document was picked for Open File…: checked here, then opened by the app on a session of its own. */
    fun openDocument(text: String, fileName: String, opener: DocumentOpener?) {
        if (opener == null) {
            host.showError("Opening a file is not available here.")
            return
        }
        val info = Json.parseObject(bridge.configFiles.inspectConfiguration(text))
        if (info?.bool("ok") != true) {
            host.showError(info?.str("error")?.ifEmpty { null } ?: "The file could not be read.")
            return
        }
        opener.open(text, fileName)
    }

    /** The templates: the shipped ones, then [userDir]'s (`template_list`). Reads files: call off the main thread. */
    fun templates(userDir: String): List<TemplateEntry> = OfflineFiles.parseTemplates(bridge.snapshots.templates(userDir))

    private val _templateChoice = MutableStateFlow<List<TemplateEntry>?>(null)

    /** The template list while Open Template… is open. */
    val templateChoice: StateFlow<List<TemplateEntry>?> = _templateChoice.asStateFlow()

    fun showTemplates(userDir: String) {
        scope.launch { _templateChoice.value = withContext(Dispatchers.IO) { templates(userDir) } }
    }

    fun closeTemplates() {
        _templateChoice.value = null
    }

    /** Open template [t] offline, as a document named after it. */
    fun openTemplate(t: TemplateEntry, userDir: String, opener: DocumentOpener?) {
        _templateChoice.value = null
        bridge.snapshots.templateText(t.id, userDir)
            .onSuccess { openDocument(it, OfflineFiles.templateFileName(t.name), opener) }
            .onFailure { host.showError(it.message ?: "The template could not be read.") }
    }

    /** The open document as it now stands, for Save File…; null after an alert. */
    suspend fun savedDocument(): String? = bridge.snapshots.saveDocument().onFailure {
        host.showError(
            it.message ?: "The file could not be saved."
        )
    }.getOrNull()

    /** "Save as Template…": the name asked for, then the template filed in [userDir]. */
    fun askSaveTemplate(userDir: java.io.File) {
        host.show(
            ConsoleAlert(
                "Save as Template",
                "The settings as they are now, as a starting point for other systems.",
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Save", AlertButtonRole.DEFAULT) { r -> saveTemplate(r.text.trim().ifEmpty { "Template" }, userDir) },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("Name", bridge.snapshots.offline.value?.name ?: "", TEMPLATE_NAME_MAX)
            )
        )
    }

    private fun saveTemplate(name: String, userDir: java.io.File) {
        scope.launch {
            val text =
                bridge.snapshots.saveAsTemplate(name).onFailure { host.showError(it.message ?: "The template was not saved.") }.getOrNull()
                    ?: return@launch
            try {
                withContext(Dispatchers.IO) {
                    userDir.mkdirs()
                    java.io.File(userDir, OfflineFiles.templateFileName(name)).writeText(text)
                }
            } catch (e: Exception) {
                writeFailed(e)
                return@launch
            }
            host.showSuccess("Template \"$name\" saved. Open it from Open Template….")
        }
    }

    // ═══ Filter files ═════════════════════════════════════════════════════

    /** The filter file text, null (after an alert) when there is none to write. */
    suspend fun filtersDocument(): String? = exported(if (bridge.connected) bridge.configFiles.exportFilters() else "")

    fun filtersExported() = host.showSuccess("Filters exported successfully")

    /**
     * A filter file was picked: the core detects the format; a REW/AutoEQ file
     * gets the single-channel picker (inputs ticked), a DSPi Console file the
     * multi-channel picker (its channels ticked). Not gated (live setters only).
     */
    fun importFilters(text: String) {
        scope.launch {
            val f = Json.parseObject(bridge.configFiles.inspectFilters(text))
            if (f?.bool("ok") != true) {
                host.showError(f?.str("error")?.ifEmpty { null } ?: "No valid filters found in file")
                return@launch
            }
            val channels = f.list("channels").orEmpty().filterIsInstance<Map<String, Any?>>()
            val prompt =
                if (f.str("kind") == "rew") {
                    PresetDocumentFiles.singleChannelPrompt(f.int("filters") ?: 0, f.double("preamp"))
                } else {
                    PresetDocumentFiles.MULTI_CHANNEL_PROMPT
                }
            val boxes =
                channels.mapNotNull { c ->
                    val wire = c.int("wire") ?: return@mapNotNull null
                    val bands = c.int("bands") ?: 0
                    AlertCheckbox(
                        wire.toString(),
                        c.str("label") ?: "Ch $wire",
                        "$bands band${if (bands == 1) "" else "s"}",
                        c.bool("checked") == true
                    )
                }
            host.show(
                ConsoleAlert(
                    "Import Filters",
                    prompt,
                    AlertStyle.INFORMATIONAL,
                    listOf(
                        AlertButton("Import", AlertButtonRole.DEFAULT) { r ->
                            applyFilters(text, r.checked.mapNotNull { it.toIntOrNull() }.sorted())
                        },
                        AlertButton("Cancel", AlertButtonRole.CANCEL)
                    ),
                    checkboxes = boxes
                )
            )
        }
    }

    private fun applyFilters(text: String, wires: List<Int>) {
        if (wires.isEmpty()) return
        scope.launchDeviceSequence {
            val r = Json.parseObject(bridge.configFiles.importFilters(text, wires))
            if (r?.bool("ok") == false || r == null) {
                host.showError(r?.str("error")?.ifEmpty { null } ?: "Failed to import filters")
                return@launchDeviceSequence
            }
            val lines = r.list("lines").orEmpty().filterIsInstance<String>()
            host.showSuccess(lines.joinToString("\n\n").ifEmpty { "Filters imported successfully" })
        }
    }

    // ═══ File errors (the SAF side) ═══════════════════════════════════════

    fun readFailed(error: Throwable) = host.showError("Failed to read file: ${error.message ?: error.javaClass.simpleName}")

    fun writeFailed(error: Throwable) = host.showError("Failed to write file: ${error.message ?: error.javaClass.simpleName}")
}

/** A template's name, as long as a snapshot's. */
internal const val TEMPLATE_NAME_MAX = 200

/** The import preview's state (WP P1AN1). [guardDefault]: the core guards here by default (DSPi Link). */
data class ImportPreview(
    val text: String,
    val name: String,
    val intro: String,
    val diff: DocDiff,
    val selected: Set<ImportBlock>,
    val guarded: Boolean,
    val guardDefault: Boolean
) {
    /** The rows of the chosen blocks: what Apply changes. */
    val chosenRows: List<DiffRow> get() = diff.rows.filter { r -> selected.any { it.key == r.block } }
}
