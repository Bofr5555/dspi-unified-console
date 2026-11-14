// WP P1AN1: the core's snapshots (P1SN), A/B compare (P1AB), safe apply
// (P1SA), import preview and per-block import (P1DF) and offline documents
// (P1OF) for the Android screens. Every rule (what a recall writes, when a
// guard reverts, what a compare keeps) is the core's; this carries the calls
// onto the bridge's FFI thread, keeps the compare's and the guard's state
// observable, and re-reads the mirror after anything that changed the device.
package se.bofr.dspi.bridge

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.core.AbCompare
import se.bofr.dspi.core.AbSide
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.core.SnapshotInfo
import se.bofr.dspi.core.SnapshotRecall

/** The open connection is a document (`offline_document`), not a device. */
data class OfflineDocument(val fileName: String, val name: String, val kind: String, val platform: String)

/** What a recall did: at once ([recall]), or with a way back ([guard], waiting for Keep or Revert). */
data class RecallOutcome(val recall: SnapshotRecall? = null, val guard: GuardedApply? = null)

/** A failed call, in words for the user (the core's sentence where it has one). */
class SnapshotsException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** [DspiBridge.snapshots]. Suspending members run on the bridge's FFI thread; call them from the main thread. */
interface Snapshots {
    /** The guarded apply's state (`apply_status`), refreshed by every guarded call and [poll]. */
    val guard: StateFlow<GuardedApply>

    /** The A/B compare (`ab_state`), refreshed by every A/B call, a guard's end and a device change. */
    val compare: StateFlow<AbCompare>

    /** The open document when the connection is one (P1OF), else null. */
    val offline: StateFlow<OfflineDocument?>

    /** True while a guarded apply waits for Keep or Revert: A/B is refused then (core, P1CFIX F4). */
    val guardWaiting: Boolean get() = guard.value.phase == GuardPhase.WAITING

    /** `snapshot_set_dir`: the app's own folder for the store. */
    suspend fun setFolder(dir: String): Result<Unit>

    /** `snapshot_list` for the open device; never fails. */
    suspend fun list(): List<SnapshotInfo>

    suspend fun capture(name: String, note: String): Result<SnapshotInfo>

    suspend fun rename(id: String, name: String): Result<SnapshotInfo>

    suspend fun setNote(id: String, note: String): Result<SnapshotInfo>

    suspend fun delete(id: String): Result<Unit>

    /** The snapshot's document, for a file. */
    suspend fun export(id: String): Result<String>

    /** File a snapshot document in the store (`snapshot_import`). */
    suspend fun importDocument(text: String, name: String, note: String): Result<SnapshotInfo>

    /**
     * `apply_guard_default`: an import or a recall is guarded here by default
     * (DSPi Link to a hub with the guard; OD-P1g). On USB it is the user's choice.
     */
    suspend fun guardDefault(): Boolean

    /**
     * Recall [id]: guarded (`apply_guarded_snapshot`, [timeoutS] 0 = the core's
     * 30 s) when [guarded], else `snapshot_recall`. Over DSPi Link a caller
     * passes [guardDefault] (OD-P1g, P1CFIX): the core never guards a plain recall itself.
     */
    suspend fun recall(id: String, guarded: Boolean, timeoutS: Long = 0): Result<RecallOutcome>

    /** Start a compare: [aId] "" captures the device as A. */
    suspend fun abStart(aId: String, bId: String): Result<AbCompare>

    suspend fun abToggle(): Result<AbCompare>

    /** P1DF's diff JSON between A and B. */
    suspend fun abDiff(): Result<String>

    suspend fun abKeep(side: AbSide): Result<AbCompare>

    /** Keep the guarded change. */
    suspend fun keep(): Result<GuardedApply>

    /** Put the settings from before back now. */
    suspend fun revert(): Result<GuardedApply>

    /**
     * Drive the guard (`apply_poll`): the timeout's revert on USB, a lost
     * connection, the hub's answer after a reconnect. Call from a poll while a
     * screen is visible (decision E6); no I/O while nothing waits.
     */
    suspend fun poll(): GuardedApply

    /** `config_preview`: what importing [text] with [options] would change; the core's JSON. */
    suspend fun importPreview(text: String, options: Long, eqChannels: Long = ALL_CHANNELS): String

    /**
     * Import [text] with the blocks [options] chooses (`config_import_selected`,
     * [DspiConstants.IMPORT_BY_BLOCK] set). GATED [DangerousOp.CONFIG_IMPORT].
     * The core's reply JSON (`{ok, error, lines, clean, ...}`).
     */
    suspend fun importSelected(token: ConfirmationToken, text: String, options: Long, eqChannels: Long = ALL_CHANNELS): String

    /** Import with a way back (`apply_guarded_import`): no token, the guard is the way back. */
    suspend fun importGuarded(text: String, options: Long, eqChannels: Long = ALL_CHANNELS, timeoutS: Long = 0): Result<GuardedApply>

    /** The open document as it now stands (`save_document`); its edits become the baseline. */
    suspend fun saveDocument(): Result<String>

    /** The open device or document as a template named [name] (`save_as_template`). */
    suspend fun saveAsTemplate(name: String): Result<String>

    // -- WP P1DCFIX -- (F8) both read files: off the caller's thread (the FFI thread)
    /** `template_list`'s JSON: the shipped templates, then [userDir]'s. No device. */
    suspend fun templates(userDir: String): String

    suspend fun templateText(id: String, userDir: String): Result<String>

    // -- WP P1DCFIX -- (F7) the countdown's words are the core's (P1NT's `guard.waiting`)
    /**
     * The sentence the countdown shows while a guard waits, with `{s}` for the
     * whole seconds left ([countdownLine]). The core's notice; its English
     * words when the core has none (a test double).
     */
    suspend fun countdownWords(): String = COUNTDOWN_WORDS

    // -- WP P1AN2 -- over DSPi Link the hub's timer is the backstop (P1HA)
    /** The sentence after the countdown when the hub holds the guard: the reason of the core's `guard.hub_backstop`, or its English words. */
    suspend fun backstopWords(): String = HUB_BACKSTOP_WORDS

    companion object {
        /** Every channel's EQ (`eq_channels` = `u32::MAX`). */
        const val ALL_CHANNELS: Long = 0xFFFF_FFFFL

        // -- WP P1DCFIX --
        /** The core's notice key for the countdown sentence. */
        const val COUNTDOWN_NOTICE_KEY = "guard.waiting"

        /** The core's `guard.waiting` title and reason, for a core without the notice table. */
        const val COUNTDOWN_WORDS = "Keep these settings? They go back in {s} s unless you press Keep."

        // -- WP P1AN2 --
        /** The core's notice key for the hub's backstop sentence (P1WB2 adds the row). */
        const val HUB_BACKSTOP_NOTICE_KEY = "guard.hub_backstop"

        /** Its words for a core without the row. */
        const val HUB_BACKSTOP_WORDS = "The hub puts them back too if this console goes away."

        /** [words] (with `{s}`) for [remainingMs]: the whole seconds left, never below 0. */
        fun countdownLine(words: String, remainingMs: Long): String =
            words.replace("{s}", ((remainingMs + 999) / 1000).coerceAtLeast(0).toString())

        val IDLE_GUARD = GuardedApply(0, GuardPhase.IDLE, "", false, 0, 0, "", "", emptyList(), emptyList())
        val NO_COMPARE = AbCompare(false, null, "", "", null)

        /** The user's words for a failure of [e]. */
        fun wordsOf(e: Throwable): String {
            val raw = when (e) {
                is DspiException.InvalidArgument -> e.msg
                is DspiException.Io -> e.msg
                is DspiException.Link -> e.msg
                is DspiException.Transfer -> e.msg
                is DspiException.Disconnected -> "No device connected"
                is DspiException.ReadOnly -> "The device is read-only"
                is DspiException.Unsupported -> "Not available here"
                is WritesNotAllowedException -> "The device does not take changes now"
                else -> e.message ?: e.javaClass.simpleName
            }.trim().trimEnd('.')
            return raw.replaceFirstChar { it.uppercase() } + "."
        }
    }

    /** For bridges without a core (test doubles): nothing is stored, every change is refused. */
    object Unavailable : Snapshots {
        private fun <T> no(): Result<T> = Result.failure(SnapshotsException("Snapshots are not available here."))

        override val guard: StateFlow<GuardedApply> = MutableStateFlow(IDLE_GUARD).asStateFlow()
        override val compare: StateFlow<AbCompare> = MutableStateFlow(NO_COMPARE).asStateFlow()
        override val offline: StateFlow<OfflineDocument?> = MutableStateFlow<OfflineDocument?>(null).asStateFlow()

        override suspend fun setFolder(dir: String): Result<Unit> = no()

        override suspend fun list(): List<SnapshotInfo> = emptyList()

        override suspend fun capture(name: String, note: String): Result<SnapshotInfo> = no()

        override suspend fun rename(id: String, name: String): Result<SnapshotInfo> = no()

        override suspend fun setNote(id: String, note: String): Result<SnapshotInfo> = no()

        override suspend fun delete(id: String): Result<Unit> = no()

        override suspend fun export(id: String): Result<String> = no()

        override suspend fun importDocument(text: String, name: String, note: String): Result<SnapshotInfo> = no()

        override suspend fun guardDefault() = false

        override suspend fun recall(id: String, guarded: Boolean, timeoutS: Long): Result<RecallOutcome> = no()

        override suspend fun abStart(aId: String, bId: String): Result<AbCompare> = no()

        override suspend fun abToggle(): Result<AbCompare> = no()

        override suspend fun abDiff(): Result<String> = no()

        override suspend fun abKeep(side: AbSide): Result<AbCompare> = no()

        override suspend fun keep(): Result<GuardedApply> = no()

        override suspend fun revert(): Result<GuardedApply> = no()

        override suspend fun poll(): GuardedApply = IDLE_GUARD

        override suspend fun importPreview(text: String, options: Long, eqChannels: Long) =
            """{"ok":false,"error":"The changes cannot be listed here."}"""

        override suspend fun importSelected(token: ConfirmationToken, text: String, options: Long, eqChannels: Long) =
            """{"ok":false,"error":"Importing is not available here."}"""

        override suspend fun importGuarded(text: String, options: Long, eqChannels: Long, timeoutS: Long): Result<GuardedApply> = no()

        override suspend fun saveDocument(): Result<String> = no()

        override suspend fun saveAsTemplate(name: String): Result<String> = no()

        override suspend fun templates(userDir: String) = "[]"

        override suspend fun templateText(id: String, userDir: String): Result<String> = no()
    }
}

internal class SnapshotsImpl(private val bridge: DspiBridgeImpl) : Snapshots {
    private val _guard = MutableStateFlow(Snapshots.IDLE_GUARD)
    override val guard: StateFlow<GuardedApply> = _guard.asStateFlow()
    private val _compare = MutableStateFlow(Snapshots.NO_COMPARE)
    override val compare: StateFlow<AbCompare> = _compare.asStateFlow()
    private val _offline = MutableStateFlow<OfflineDocument?>(null)
    override val offline: StateFlow<OfflineDocument?> = _offline.asStateFlow()

    private val core get() = bridge.core

    init {
        // A device opened or gone: the core ends a compare on a disconnect and
        // knows whether the connection is a document; read both (no I/O)
        bridge.scope.launch(bridge.main) {
            bridge.deviceGeneration.collect { refreshStates() }
        }
    }

    private suspend fun refreshStates() {
        // No I/O and never failing in the core; a test double without them reads as none
        val s =
            bridge.onFfi {
                Triple(
                    runCatching { core.abState() }.getOrDefault(Snapshots.NO_COMPARE),
                    runCatching { core.applyStatus() }.getOrDefault(Snapshots.IDLE_GUARD),
                    runCatching { offlineOf(core.offlineDocument()) }.getOrNull()
                )
            }
        _compare.value = s.first
        _guard.value = s.second
        _offline.value = s.third
    }

    private fun offlineOf(json: String): OfflineDocument? {
        val m = Json.parseObject(json) ?: return null
        if (m.bool("offline") != true) return null
        return OfflineDocument(m.str("fileName").orEmpty(), m.str("name").orEmpty(), m.str("kind").orEmpty(), m.str("platform").orEmpty())
    }

    /** Runs [block] on the FFI thread; a core error becomes a failure in the user's words. */
    private suspend fun <T> call(block: () -> T): Result<T> = bridge.onFfi {
        try {
            Result.success(block())
        } catch (e: DspiException) {
            Result.failure(SnapshotsException(lockWords(e) ?: Snapshots.wordsOf(e), e))
        }
    }

    // -- WP MERGEP1D -- a refusal by the installer's lock in its notice row
    // (`installer_lock.hub` when the hub holds the lock, not the "on this
    // console" words); the table only, no I/O. The one lock prompt
    // (LockState.heldBy) comes with P1-E.
    private fun lockWords(e: DspiException): String? = (e as? DspiException.InstallerLock)?.let {
        runCatching { core.noticeFor(it) }.getOrNull()
            ?.takeIf { n -> n.key.startsWith("installer_lock") }
            ?.let { n -> "${n.title}: ${n.reason}" }
    }

    /** A write refused by G1 (read-only, a viewer, no device): the bridge reports it; nothing is sent. */
    private fun refused(name: String): Result<Nothing>? = if (bridge.writeRefused("snapshots.$name")) {
        Result.failure(SnapshotsException(Snapshots.wordsOf(WritesNotAllowedException(""))))
    } else {
        null
    }

    /** The device changed as a whole: the mirror, the compare, the guard. */
    private suspend fun changed() {
        bridge.refreshAll()
        refreshStates()
    }

    override suspend fun setFolder(dir: String) = call { core.snapshotSetDir(dir) }

    override suspend fun list(): List<SnapshotInfo> = bridge.onFfi { runCatching { core.snapshotList("") }.getOrDefault(emptyList()) }

    override suspend fun capture(name: String, note: String) = call { core.snapshotCapture(name, note) }

    override suspend fun rename(id: String, name: String) = call { core.snapshotRename(id, name) }

    override suspend fun setNote(id: String, note: String) = call { core.snapshotSetNote(id, note) }

    override suspend fun delete(id: String) = call { core.snapshotDelete(id) }

    override suspend fun export(id: String) = call { core.snapshotExport(id) }

    override suspend fun importDocument(text: String, name: String, note: String) = call { core.snapshotImport(text, name, note) }

    override suspend fun guardDefault(): Boolean = bridge.onFfi { runCatching { core.applyGuardDefault() }.getOrDefault(false) }

    override suspend fun recall(id: String, guarded: Boolean, timeoutS: Long): Result<RecallOutcome> {
        refused("recall")?.let { return it }
        val r =
            call {
                if (guarded) {
                    RecallOutcome(
                        guard = core.applyGuardedSnapshot(id, timeoutS)
                    )
                } else {
                    RecallOutcome(recall = core.snapshotRecall(id))
                }
            }
        changed()
        return r
    }

    override suspend fun abStart(aId: String, bId: String): Result<AbCompare> {
        refused("abStart")?.let { return it }
        return call { core.abStart(aId, bId) }.also { changed() }
    }

    override suspend fun abToggle(): Result<AbCompare> {
        refused("abToggle")?.let { return it }
        return call { core.abToggle() }.also { changed() }
    }

    override suspend fun abDiff() = call { core.abDiff() }

    override suspend fun abKeep(side: AbSide): Result<AbCompare> {
        refused("abKeep")?.let { return it }
        return call { core.abKeep(side) }.also { changed() }
    }

    // Keep and Revert are never refused: a revert restores a state captured before (P1LM's rule)
    override suspend fun keep() = call { core.applyConfirm() }.also { ended(it) }

    override suspend fun revert() = call { core.applyRevert() }.also { ended(it) }

    private suspend fun ended(r: Result<GuardedApply>) {
        r.getOrNull()?.let { _guard.value = it }
        changed()
    }

    override suspend fun poll(): GuardedApply {
        val before = _guard.value
        val now = bridge.onFfi { core.applyPoll() }
        _guard.value = now
        // The guard ended here (timeout, lost connection, the hub's answer): the device may have moved
        if (before.phase == GuardPhase.WAITING && now.phase != GuardPhase.WAITING) changed()
        return now
    }

    override suspend fun importPreview(text: String, options: Long, eqChannels: Long): String = bridge.onFfi {
        // The preview never stops an import: without it the blocks are still offered
        runCatching { core.configPreview(text, options or DspiConstants.IMPORT_BY_BLOCK.toLong(), eqChannels) }
            .getOrElse { Json.write(mapOf("ok" to false, "error" to "The changes could not be listed: ${Snapshots.wordsOf(it)}")) }
    }

    override suspend fun importSelected(token: ConfirmationToken, text: String, options: Long, eqChannels: Long): String {
        if (bridge.writeRefused("snapshots.importSelected", token, DangerousOp.CONFIG_IMPORT)) return READ_ONLY_REPLY
        bridge.requireToken(token, DangerousOp.CONFIG_IMPORT)
        if (!bridge.connected) return """{"ok":false,"error":"No device connected."}"""
        val r =
            bridge.onFfi {
                try {
                    // -- WP P1AN2 -- P1TK: the core's token for the import the user confirmed
                    core.configImportConfirmed(
                        text,
                        options or DspiConstants.IMPORT_BY_BLOCK.toLong(),
                        eqChannels,
                        bridge.coreToken(DangerousOp.CONFIG_IMPORT)
                    )
                } finally {
                    bridge.flashWritten()
                }
            }
        afterImport(Json.parseObject(r))
        return r
    }

    override suspend fun importGuarded(text: String, options: Long, eqChannels: Long, timeoutS: Long): Result<GuardedApply> {
        refused("importGuarded")?.let { return it }
        val r = call { core.applyGuardedImport(text, options or DspiConstants.IMPORT_BY_BLOCK.toLong(), eqChannels, timeoutS) }
        r.getOrNull()?.let { _guard.value = it }
        changed()
        return r
    }

    /** As [ConfigFiles.importConfiguration]: the input-pair links and the unsaved hardware mark. */
    private suspend fun afterImport(m: Map<String, Any?>?) {
        if (m?.bool("ok") == true) {
            m.long("links")?.let { bridge.restoreInputLinks(it.toInt()) }
            if (m.bool("hardwareWritten") == true) bridge.markHardwareUnsaved()
        }
        changed()
    }

    override suspend fun saveDocument(): Result<String> {
        val links = bridge.inputLinks
        return call { core.saveDocument(bridge.appVersion, links) }.also { bridge.refreshAll() }
    }

    override suspend fun saveAsTemplate(name: String): Result<String> {
        val links = bridge.inputLinks
        return call { core.saveAsTemplate(name, bridge.appVersion, links) }
    }

    // -- WP P1DCFIX -- (F8) on the FFI thread, never the caller's
    override suspend fun templates(userDir: String): String = bridge.onFfi { runCatching { core.templateList(userDir) }.getOrDefault("[]") }

    override suspend fun templateText(id: String, userDir: String): Result<String> = call { core.templateText(id, userDir) }

    override suspend fun countdownWords(): String = bridge.onFfi {
        runCatching { core.noticeForKey(Snapshots.COUNTDOWN_NOTICE_KEY, "") }.getOrNull()
            ?.takeIf { it.key == Snapshots.COUNTDOWN_NOTICE_KEY }
            ?.let { "${it.title} ${it.reason}".trim() }
            ?.takeIf { "{s}" in it }
            ?: Snapshots.COUNTDOWN_WORDS
    }

    // -- WP P1AN2 --
    override suspend fun backstopWords(): String = bridge.onFfi {
        runCatching { core.noticeForKey(Snapshots.HUB_BACKSTOP_NOTICE_KEY, "") }.getOrNull()
            ?.takeIf { it.key == Snapshots.HUB_BACKSTOP_NOTICE_KEY }
            // -- WP P1EFIX -- (review p1e-a F1) the reason alone, as the web and Qt
            ?.reason?.trim()
            ?.takeIf { it.isNotEmpty() }
            ?: Snapshots.HUB_BACKSTOP_WORDS
    }

    private companion object {
        const val READ_ONLY_REPLY = """{"ok":false,"error":"The device does not take changes now."}"""
    }
}
