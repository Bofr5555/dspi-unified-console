// WP P1AN1: the Snapshots screen's logic (PRO1 W3, W4, W6), JVM-tested. The
// store, the recall, the compare and the guard are the core's (P1SN, P1AB,
// P1SA) through DspiBridge.snapshots; this holds the list, the user's choices
// and the one-line results. A success shows as the change itself (a list row,
// the A/B chip, the countdown bar): no success alert.
package se.bofr.dspi.feature.presets.snapshots

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HubServer
import se.bofr.dspi.core.AbCompare
import se.bofr.dspi.core.AbSide
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.core.HubCaps
import se.bofr.dspi.core.MergedSnapshot
import se.bofr.dspi.core.SnapshotInfo
import se.bofr.dspi.core.SnapshotRecall
import se.bofr.dspi.feature.presets.DocDiff
import se.bofr.dspi.ui.alert.AlertButton
import se.bofr.dspi.ui.alert.AlertButtonRole
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertStyle
import se.bofr.dspi.ui.alert.AlertTextField
import se.bofr.dspi.ui.alert.ConsoleAlert

/**
 * Simple shows the snapshots read-mostly: recall and A/B (WP P1DCFIX, as Qt and
 * the web: no capture, rename, note, delete, import, export, rollback choice
 * or timeout). Advanced edits them. docs/UI-MODES.md; the mode switch itself
 * is P1AN2's, which passes the device's mode here.
 */
enum class SnapshotsMode { SIMPLE, ADVANCED }

// -- WP P2AN --
/** How one of this console's snapshots stands to the hub's store. */
enum class LocalMark {
    /** The hub holds the same snapshot. */
    ON_HUB_TOO,

    /** The hub holds a different one under this id (the hub's wins; this is kept as a local copy). */
    LOCAL_COPY,

    /** Not on the hub: "Save to the hub" offered. */
    HERE_ONLY
}

/** The A/B chooser's state: which snapshot is A ("" = the device as it is now) and which is B. */
data class AbChoice(val aId: String = "", val bId: String = "")

class SnapshotsModel(
    val bridge: DspiBridge,
    private val scope: CoroutineScope,
    /** The app's own folder for the store; null: none (tests that never write). */
    private val folder: String?
) {
    val host = AlertHost()
    private val snaps get() = bridge.snapshots

    private val _items = MutableStateFlow<List<SnapshotInfo>>(emptyList())

    /** The open device's snapshots, newest first. */
    val items: StateFlow<List<SnapshotInfo>> = _items.asStateFlow()

    private val _busy = MutableStateFlow(false)

    /** A call is running: the buttons wait (one at a time, as the core's lock). */
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    private val _result = MutableStateFlow("")

    /** The last action's outcome in one line ("" none): shown under the list, not as an alert. */
    val result: StateFlow<String> = _result.asStateFlow()

    private val _diff = MutableStateFlow<DocDiff?>(null)

    /** A against B (P1DF's rows) while the compare's diff is open. */
    val diff: StateFlow<DocDiff?> = _diff.asStateFlow()

    private val _choice = MutableStateFlow(AbChoice())
    val choice: StateFlow<AbChoice> = _choice.asStateFlow()

    private val _safeRecall = MutableStateFlow(false)

    /** USB: the user asked for a way back on recalls (OD-P1g: their choice there). */
    val safeRecall: StateFlow<Boolean> = _safeRecall.asStateFlow()

    private val _guardDefault = MutableStateFlow(false)

    /** Recalls are guarded here anyway (DSPi Link to a hub with the guard; OD-P1g, P1CFIX). */
    val guardDefault: StateFlow<Boolean> = _guardDefault.asStateFlow()

    val compare: StateFlow<AbCompare> get() = snaps.compare
    val guard: StateFlow<GuardedApply> get() = snaps.guard

    private var folderSet = false

    // -- WP P1DCFIX --
    private val _timeoutS = MutableStateFlow(0)

    /** The safe apply's timeout for a guarded recall, seconds (0 = the core's 30 s); the app's setting. */
    val timeoutS: StateFlow<Int> = _timeoutS.asStateFlow()

    fun setTimeoutS(s: Int) {
        _timeoutS.value = s.coerceAtLeast(0)
    }

    // -- WP P2AN -- the hub's section (PLAN-P2 OD-P1b: the hub's copy wins; the core merges both sides)
    private val hubServer get() = bridge.hubServer

    /** The hub's families: the section shows only with `hub_snapshots`. */
    val hubCaps: StateFlow<HubCaps> get() = hubServer.caps

    private val _hubItems = MutableStateFlow<List<MergedSnapshot>>(emptyList())

    /** "On the hub": the hub's snapshots of the open device, newest first (`where` hub or both). */
    val hubItems: StateFlow<List<MergedSnapshot>> = _hubItems.asStateFlow()

    private val _localMarks = MutableStateFlow<Map<String, LocalMark>>(emptyMap())

    /** How each of this console's snapshots stands to the hub's (only while the hub's list was read). */
    val localMarks: StateFlow<Map<String, LocalMark>> = _localMarks.asStateFlow()

    private val _hubLine = MutableStateFlow("")

    /** Why the hub's list is not shown ("" when it is, or with no such hub): the core's notice row. */
    val hubLine: StateFlow<String> = _hubLine.asStateFlow()

    init {
        // Another console (or this one) changed the hub's snapshots: read both sides again
        scope.launch { hubServer.snapshotsChanged.collect { if (folderSet || folder == null) reload() } }
    }

    /** Name the store's folder (once) and read the list. */
    fun load() = once {
        if (!folderSet && folder != null) {
            snaps.setFolder(folder).onFailure { show(it) }.onSuccess { folderSet = true }
        }
        _guardDefault.value = snaps.guardDefault()
        reload()
    }

    private suspend fun reload() {
        _items.value = snaps.list().sortedByDescending { it.savedMs }
        reloadHub() // -- WP P2AN --
        val ids = _items.value.map { it.id }.toSet()
        _choice.value = _choice.value.let { c -> AbChoice(c.aId.takeIf { it in ids }.orEmpty(), c.bId.takeIf { it in ids }.orEmpty()) }
    }

    fun setSafeRecall(on: Boolean) {
        _safeRecall.value = on
    }

    fun chooseA(id: String) {
        _choice.value = _choice.value.copy(aId = id)
    }

    fun chooseB(id: String) {
        _choice.value = _choice.value.copy(bId = id)
    }

    /** "Capture…": the name (and nothing else) asked for; the note can be added later. */
    fun askCapture() {
        host.show(
            ConsoleAlert(
                "Capture Snapshot",
                "The device's settings as they are now, kept on this console.",
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Capture", AlertButtonRole.DEFAULT) { r -> capture(r.text.trim().ifEmpty { defaultName() }, "") },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("Name", defaultName(), NAME_MAX)
            )
        )
    }

    private fun defaultName() = "Snapshot ${_items.value.size + 1}"

    fun capture(name: String, note: String) = once {
        snaps.capture(name, note).onSuccess {
            _result.value = "Captured \"${it.name}\"."
            reload()
        }.onFailure(::show)
    }

    fun askRename(s: SnapshotInfo) {
        host.show(
            ConsoleAlert(
                "Rename Snapshot",
                "",
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Rename", AlertButtonRole.DEFAULT) { r ->
                        val name = r.text.trim()
                        if (name.isNotEmpty()) once { snaps.rename(s.id, name).onSuccess { reload() }.onFailure(::show) }
                    },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("Name", s.name, NAME_MAX)
            )
        )
    }

    fun askNote(s: SnapshotInfo) {
        host.show(
            ConsoleAlert(
                "Snapshot Note",
                s.name,
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Save", AlertButtonRole.DEFAULT) { r ->
                        once { snaps.setNote(s.id, r.text.trim()).onSuccess { reload() }.onFailure(::show) }
                    },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("Note", s.note, NOTE_MAX)
            )
        )
    }

    fun askDelete(s: SnapshotInfo) {
        host.show(
            ConsoleAlert(
                "Delete \"${s.name}\"?",
                "The snapshot is removed from this console. The device keeps its settings.",
                AlertStyle.WARNING,
                listOf(
                    AlertButton("Delete", AlertButtonRole.DESTRUCTIVE) {
                        once { snaps.delete(s.id).onSuccess { reload() }.onFailure(::show) }
                    },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                )
            )
        )
    }

    /**
     * Recall [s]. Guarded (a countdown with Keep and Revert) when the core
     * guards here by default (DSPi Link) or the user asked for it on USB; a plain
     * recall otherwise. Never while a guarded apply waits.
     */
    fun recall(s: SnapshotInfo) = once {
        if (snaps.guardWaiting) {
            _result.value = FINISH_SAFE_APPLY
            return@once
        }
        recallNow(s)
    }

    private suspend fun recallNow(s: SnapshotInfo) {
        val guarded = snaps.guardDefault() || _safeRecall.value
        snaps.recall(s.id, guarded, _timeoutS.value.toLong()).onSuccess { o ->
            _result.value = o.recall?.let { recalledLine(it) } ?: "Recalled \"${s.name}\". Keep or Revert it in the bar below."
        }.onFailure(::show)
    }

    // -- WP P2AN --
    private suspend fun reloadHub() {
        if (!hubServer.caps.value.hubSnapshots) {
            _hubItems.value = emptyList()
            _localMarks.value = emptyMap()
            _hubLine.value = ""
            return
        }
        val m = hubServer.mergedSnapshots()
        _hubItems.value = hubSide(m.entries)
        _localMarks.value = if (m.hubListed) marksOf(m.entries) else emptyMap()
        _hubLine.value = if (m.hubListed || m.hubNoticeKey.isEmpty()) "" else hubServer.noticeLine(m.hubNoticeKey) ?: HUB_UNREAD
    }

    /** Read the hub's side again (the caps arrived after the list). */
    fun refreshHub() = once { reloadHub() }

    /** "Save to the hub": this console's [s], under its id. */
    fun saveToHub(s: SnapshotInfo) = once {
        hubServer.saveToHub(s.id).onSuccess { p ->
            _result.value = if (p.created) "Saved \"${s.name}\" to the hub." else "The hub already holds \"${s.name}\"."
            reload()
        }.onFailure(::show)
    }

    /** "Copy here": the hub's [s] into this console's own store. */
    fun copyHere(s: SnapshotInfo) = once {
        hubServer.copyHere(s.id).onSuccess {
            _result.value = "Copied \"${it.name}\" here."
            reload()
        }.onFailure(::show)
    }

    /**
     * Recall the hub's [m]: this console's copy when it holds the same one,
     * else the hub's copied here first; then as [recall] (the same way back).
     */
    fun recallFromHub(m: MergedSnapshot) = once {
        if (snaps.guardWaiting) {
            _result.value = FINISH_SAFE_APPLY
            return@once
        }
        val here = if (m.location == HubServer.WHERE_BOTH) Result.success(m.snapshot) else hubServer.copyHere(m.snapshot.id)
        here.onSuccess { s ->
            recallNow(s)
            reload()
        }.onFailure(::show)
    }

    fun askRenameOnHub(s: SnapshotInfo) {
        host.show(
            ConsoleAlert(
                "Rename on the Hub",
                "",
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Rename", AlertButtonRole.DEFAULT) { r ->
                        val name = r.text.trim()
                        if (name.isNotEmpty()) once { hubServer.renameOnHub(s.id, name).onSuccess { reload() }.onFailure(::show) }
                    },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("Name", s.name, NAME_MAX)
            )
        )
    }

    fun askNoteOnHub(s: SnapshotInfo) {
        host.show(
            ConsoleAlert(
                "Snapshot Note on the Hub",
                s.name,
                AlertStyle.INFORMATIONAL,
                listOf(
                    AlertButton("Save", AlertButtonRole.DEFAULT) { r ->
                        once { hubServer.noteOnHub(s.id, r.text.trim()).onSuccess { reload() }.onFailure(::show) }
                    },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                ),
                textField = AlertTextField("Note", s.note, NOTE_MAX)
            )
        )
    }

    fun askDeleteFromHub(s: SnapshotInfo) {
        host.show(
            ConsoleAlert(
                "Delete \"${s.name}\" from the hub?",
                "The snapshot is removed from the hub for every console. A copy on this console stays.",
                AlertStyle.WARNING,
                listOf(
                    AlertButton("Delete", AlertButtonRole.DESTRUCTIVE) {
                        once { hubServer.deleteFromHub(s.id).onSuccess { reload() }.onFailure(::show) }
                    },
                    AlertButton("Cancel", AlertButtonRole.CANCEL)
                )
            )
        )
    }

    /** Start A/B with the chosen pair; A "" captures the device as it is now. */
    fun startCompare() = once {
        val c = _choice.value
        if (c.bId.isEmpty()) return@once
        snaps.abStart(c.aId, c.bId).onSuccess { _result.value = "Comparing: on A." }.onFailure(::show)
    }

    /** The A/B toggle: refused by the core while a guarded apply waits (the button is off then). */
    fun toggle() = once {
        if (snaps.guardWaiting) {
            _result.value = FINISH_SAFE_APPLY
            return@once
        }
        snaps.abToggle().onSuccess { c ->
            _result.value = "On ${c.side?.name ?: "?"}" + (c.recall?.let { ": ${changesLine(it.changes.size)}." } ?: ".")
        }.onFailure(::show)
    }

    fun keep(side: AbSide) = once {
        snaps.abKeep(side).onSuccess { _result.value = "Kept ${side.name}. The compare has ended." }.onFailure(::show)
        _diff.value = null
    }

    fun showDiff() = once {
        snaps.abDiff().onSuccess { _diff.value = DocDiff.parse(it) }.onFailure(::show)
    }

    fun hideDiff() {
        _diff.value = null
    }

    /** The snapshot's document, for a file; null after an alert. */
    suspend fun exportText(s: SnapshotInfo): String? = snaps.export(s.id).onFailure(::show).getOrNull()

    /** A snapshot file was picked: file it in the store. */
    fun importText(text: String) = once {
        snaps.importDocument(text, "", "").onSuccess {
            _result.value = "Added \"${it.name}\"."
            reload()
        }.onFailure(::show)
    }

    fun clearResult() {
        _result.value = ""
    }

    private fun show(e: Throwable) {
        host.showError(e.message ?: "That did not work.")
    }

    /** One call at a time, on [scope]; [busy] while it runs. */
    private fun once(block: suspend () -> Unit) {
        if (_busy.value) return
        _busy.value = true
        scope.launch {
            try {
                block()
            } finally {
                _busy.value = false
            }
        }
    }

    companion object {
        const val NAME_MAX = 200
        const val NOTE_MAX = 4000

        /** The core's refusal while a guard waits, in the plan's words (P1CFIX). */
        const val FINISH_SAFE_APPLY = "Finish the safe apply first: Keep or Revert."

        // -- WP P2AN --
        /** The hub's list could not be read and the core gave no row. */
        const val HUB_UNREAD = "The hub's snapshots could not be read."

        /** The hub's rows of a merged list (its own, not this console's local copies), newest first. */
        fun hubSide(entries: List<MergedSnapshot>): List<MergedSnapshot> =
            entries.filter { !it.localCopy && it.location != HubServer.WHERE_LOCAL }.sortedByDescending { it.snapshot.savedMs }

        /** Each of this console's snapshots against the hub's: the same there, a differing local copy, or here only. */
        fun marksOf(entries: List<MergedSnapshot>): Map<String, LocalMark> = entries.mapNotNull { e ->
            when {
                e.localCopy -> e.snapshot.id to LocalMark.LOCAL_COPY
                e.location == HubServer.WHERE_BOTH -> e.snapshot.id to LocalMark.ON_HUB_TOO
                e.location == HubServer.WHERE_LOCAL -> e.snapshot.id to LocalMark.HERE_ONLY
                else -> null
            }
        }.toMap()

        fun changesLine(n: Int) = if (n == 1) "1 change" else "$n changes"

        fun recalledLine(r: SnapshotRecall): String {
            val base = "Recalled \"${r.name}\": ${changesLine(r.changes.size)}."
            return if (r.notes.isEmpty()) base else "$base ${r.notes.joinToString(" ")}"
        }
    }
}
