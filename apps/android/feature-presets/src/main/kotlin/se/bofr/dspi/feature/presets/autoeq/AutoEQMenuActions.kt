// Port of DSPi-Console (macOS) `DSPi Console/DSPi_ConsoleApp.swift` @ v1.1.6-beta5
// `AutoEQMenuActions` (Update Database…: Import File…, Reset to Built-in) and
// the AutoEQ menu's Favorite Profiles / Clear Favorites. "Rebuild from GitHub"
// is not offered (PRESETS deviation 2). WP AN18: "Download..." fetches a
// ready-made database from an address (macOS `updateDatabase(from:)`) through
// CG6's `autoeq_update`, with [HttpAutoEqFetch] as the host's fetch.
package se.bofr.dspi.feature.presets.autoeq

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.ui.alert.AlertButton
import se.bofr.dspi.ui.alert.AlertButtonRole
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertStyle
import se.bofr.dspi.ui.alert.AlertTextField
import se.bofr.dspi.ui.alert.ConsoleAlert

class AutoEQMenuActions(
    private val manager: AutoEQManager,
    private val bridge: DspiBridge,
    private val scope: CoroutineScope,
    private val host: AlertHost,
    /** Where database files are read and parsed (Dispatchers.IO in the app). */
    private val io: CoroutineDispatcher,
    // -- WP AN18 --
    private val fetch: AutoEqFetch = HttpAutoEqFetch()
) {
    /** Favorite Profiles > entry, and the browser's Apply: applies immediately (live, unsaved). */
    fun applyProfile(entry: HeadphoneEntry): Boolean {
        if (!manager.applyProfile(entry, bridge)) {
            host.showError("No device connected.")
            return false
        }
        return true
    }

    /** Parses the database on [io] if it isn't yet (the browser opening, a favourite, Update Database). */
    fun loadDatabase() {
        scope.launch { withContext(io) { manager.ensureLoaded() } }
    }

    /** Favorite Profiles > entry: loads the database if needed, then applies. */
    fun applyFavorite(favorite: FavoriteProfile) {
        scope.launch {
            withContext(io) { manager.ensureLoaded() }
            val entry = manager.entry(favorite.id)
            if (entry == null) {
                host.showError("${favorite.name} is not in the AutoEQ database.")
                return@launch
            }
            applyProfile(entry)
        }
    }

    /** "Update Database...": shows the current database and the update methods. */
    fun updateDatabase(onImportFile: () -> Unit) {
        scope.launch {
            withContext(io) { manager.ensureLoaded() }
            showUpdateDatabase(onImportFile)
        }
    }

    private fun showUpdateDatabase(onImportFile: () -> Unit) {
        val buttons =
            buildList {
                add(AlertButton("Import File...", AlertButtonRole.DEFAULT) { onImportFile() })
                // -- WP AN18 --
                if (manager.userDatabasePath != null) add(AlertButton("Download...", AlertButtonRole.NORMAL) { askDownloadAddress() })
                if (manager.hasUserDatabase) add(AlertButton("Reset to Built-in", AlertButtonRole.NORMAL) { resetToBuiltIn() })
                add(AlertButton("Cancel", AlertButtonRole.CANCEL))
            }
        host.show(
            ConsoleAlert(
                "Update AutoEQ Database",
                "Current database: ${manager.databaseDate.value ?: "Unknown"}\nEntries: ${manager.entries.value.size}\n\nChoose an update method:",
                AlertStyle.INFORMATIONAL,
                buttons
            )
        )
    }

    /** "Import File...": an `autoeq_database.json` was picked and read. */
    fun importDatabase(text: String) {
        scope.launch {
            try {
                withContext(io) { manager.updateDatabaseFromText(text) }
                host.showSuccess("Database updated successfully.\nEntries: ${manager.entries.value.size}")
            } catch (e: Exception) {
                host.showError("Failed to import database: ${e.message ?: e.javaClass.simpleName}")
            }
        }
    }

    // -- WP AN18 --

    private fun askDownloadAddress() {
        host.show(
            ConsoleAlert(
                "Download AutoEQ Database",
                "The address of a ready-made autoeq_database.json (https). It replaces the database in use once it has been checked.",
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Download", AlertButtonRole.DEFAULT) { downloadDatabase(it.text) },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("https://", "", maxLength = URL_MAX_LENGTH)
            )
        )
    }

    /** "Download": the core fetches through [fetch], checks the answer and writes the user copy; then it is read. */
    fun downloadDatabase(address: String) {
        val path = manager.userDatabasePath ?: return host.showError("There is nowhere to store a database.")
        if (HttpAutoEqFetch.parse(address) == null) return host.showError("That is not an https:// address.")
        scope.launch {
            host.beginProgress("Downloading database...")
            val info = try {
                withContext(io) {
                    bridge.coreTools.autoeqUpdate(path, address.trim(), fetch).also { manager.reloadAfterUpdate() }
                }
            } catch (e: DspiException) {
                return@launch host.showError("Failed to download database: ${downloadError(e)}")
            } finally {
                // Whatever ends the download, the overlay goes with it
                host.endProgress()
            }
            host.showSuccess("Database updated successfully.\nEntries: ${info.entryCount}")
        }
    }

    private fun resetToBuiltIn() {
        scope.launch {
            try {
                withContext(io) { manager.resetToBuiltInDatabase() }
                host.showSuccess("Reset to built-in database.\nEntries: ${manager.entries.value.size}")
            } catch (e: Exception) {
                host.showError("Failed to reset: ${e.message ?: e.javaClass.simpleName}")
            }
        }
    }

    companion object {
        /** Long enough for any GitHub raw address. */
        const val URL_MAX_LENGTH = 2048

        /** What went wrong, for the alert: the core's message, or why no answer came. */
        fun downloadError(e: DspiException): String = when (e) {
            is DspiException.AutoEq -> e.msg
            is DspiException.Transfer -> "no answer from the server."
            is DspiException.Unsupported -> "not available in this build."
            else -> e.message ?: e.javaClass.simpleName
        }
    }
}
