// WP P1AN2: the core's Installer and safety calls for the Android screens:
// the installer's PIN and lock (P1LK, the hub's through P1HL), the limits
// profile (P1LM), the mode manifest and its resolution (P1LK), the notice
// table (P1NT), protect all outputs and the S2 warning (P1PR) and the AutoEQ
// preview and apply to a channel pair (P1AQ). Every rule is the core's; this
// carries the calls onto the bridge's FFI thread (the PIN's hash takes 0.1 to
// 1 s: never the UI thread), keeps the lock and the limits observable, and
// re-reads the mirror after anything that wrote the device.
package se.bofr.dspi.bridge

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.core.AutoEqApply
import se.bofr.dspi.core.AutoEqPreview
import se.bofr.dspi.core.AutoEqProfile
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LimitFieldRange
import se.bofr.dspi.core.LimitsStatus
import se.bofr.dspi.core.LockState
import se.bofr.dspi.core.ModeChoice
import se.bofr.dspi.core.ModeManifest
import se.bofr.dspi.core.Notice

/** A failed Installer call, in words for the user (the core's notice where it has one). */
class InstallerException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** One output's limits in a profile (`limits.outputs[]`); null limits nothing. */
data class OutputLimits(
    val output: Int,
    val maxGainDb: Float? = null,
    val highpassMinHz: Float? = null,
    val highpassMinSlopeDb: Int? = null,
    val delayMinMs: Float? = null,
    val delayMaxMs: Float? = null,
    val limiterCeilingDb: Float? = null
) {
    val empty: Boolean
        get() = listOf(maxGainDb, highpassMinHz, highpassMinSlopeDb, delayMinMs, delayMaxMs, limiterCeilingDb).all { it == null }
}

/**
 * The limits profile as the Installer page edits it (`docs/integrator/limits-profiles.md`):
 * every field may be left out (null), and one left out limits nothing.
 */
data class LimitsDraft(
    val outputs: List<OutputLimits> = emptyList(),
    val maxPeqBoostDb: Float? = null,
    val masterVolumeMaxDb: Float? = null,
    val lockedPages: List<String> = emptyList(),
    val note: String = "",
    val contact: String = ""
) {
    /** Output [o]'s limits (none set: an empty row). */
    fun output(o: Int): OutputLimits = outputs.firstOrNull { it.output == o } ?: OutputLimits(o)

    /** With output [row]'s limits replaced (an empty row is dropped). */
    fun withOutput(row: OutputLimits): LimitsDraft =
        copy(outputs = (outputs.filter { it.output != row.output } + listOfNotNull(row.takeUnless { it.empty })).sortedBy { it.output })

    /** The profile document `limits_apply_profile` takes. */
    fun toDocument(serial: String): String {
        val limits = LinkedHashMap<String, Any?>()
        limits["serial"] = serial
        limits["outputs"] =
            outputs.filterNot { it.empty }.map { o ->
                LinkedHashMap<String, Any?>().apply {
                    put("output", o.output)
                    o.maxGainDb?.let { put("maxGainDb", it) }
                    o.highpassMinHz?.let { put("highpassMinHz", it) }
                    o.highpassMinSlopeDb?.let { put("highpassMinSlopeDb", it) }
                    o.delayMinMs?.let { put("delayMinMs", it) }
                    o.delayMaxMs?.let { put("delayMaxMs", it) }
                    o.limiterCeilingDb?.let { put("limiterCeilingDb", it) }
                }
            }
        maxPeqBoostDb?.let { limits["maxPeqBoostDb"] = it }
        masterVolumeMaxDb?.let { limits["masterVolumeMaxDb"] = it }
        limits["lockedPages"] = lockedPages
        if (note.isNotEmpty()) limits["note"] = note
        if (contact.isNotEmpty()) limits["contact"] = contact
        return Json.write(
            mapOf(
                "schemaVersion" to 1,
                "meta" to mapOf("kind" to "limits", "name" to "Limits profile"),
                "limits" to limits
            )
        )
    }

    companion object {
        /** The draft in a profile document (its `limits` block, or `meta.limits`); null when there is none. */
        fun fromDocument(text: String): LimitsDraft? {
            val root = Json.parseObject(text) ?: return null
            val l = root.obj("limits") ?: root.obj("meta")?.obj("limits") ?: return null
            val outs =
                l.list("outputs").orEmpty().filterIsInstance<Map<String, Any?>>().mapNotNull { m ->
                    val o = m.int("output") ?: return@mapNotNull null
                    OutputLimits(
                        o,
                        m.float("maxGainDb"),
                        m.float("highpassMinHz"),
                        m.int("highpassMinSlopeDb"),
                        m.float("delayMinMs"),
                        m.float("delayMaxMs"),
                        m.float("limiterCeilingDb")
                    )
                }
            return LimitsDraft(
                outputs = outs.sortedBy { it.output },
                maxPeqBoostDb = l.float("maxPeqBoostDb"),
                masterVolumeMaxDb = l.float("masterVolumeMaxDb"),
                lockedPages = l.list("lockedPages").orEmpty().filterIsInstance<String>(),
                note = l.str("note").orEmpty(),
                contact = l.str("contact").orEmpty()
            )
        }
    }
}

/** What applying a profile did: the import-style `lines`, the values held to it (`held`), and writes that failed (`notes`). */
data class LimitsReport(val lines: List<String>, val held: List<String>, val notes: List<String>) {
    /** One line for the notice line: the clamp report, or that nothing had to move. */
    val summary: String
        get() = (held.ifEmpty { lines } + notes).joinToString(" ").ifEmpty { "Limits saved. Nothing had to change." }
}

/** [DspiBridge.installer]. Suspending members run on the bridge's FFI thread; call them from the main thread. */
interface Installer {
    /** The open device's installer lock (`installer_lock_state`); null before the first read or with no core. */
    val lock: StateFlow<LockState?>

    /** `limits_status`; null before the first read. */
    val limits: StateFlow<LimitsStatus?>

    /** The profile in force, as a draft; null with none. */
    val profile: StateFlow<LimitsDraft?>

    /** `limits_set_dir`: the app's folder for the profiles and the PIN (name it before a device opens). */
    suspend fun setFolder(dir: String): Result<Unit>

    /** Re-read the lock and the limits; [hub] also asks a DSPi Link hub (`installer_lock_refresh`, a round trip). */
    suspend fun refresh(hub: Boolean = true)

    suspend fun unlock(pin: String): Result<LockState>

    suspend fun lockNow(): Result<LockState>

    /** Set the PIN; the result is the recovery code, shown this once (`installer_lock.recovery`). */
    suspend fun setPin(pin: String): Result<String>

    suspend fun clearPin(): Result<LockState>

    /** What the lock covers (manifest screen ids); empty: a PIN alone locks nothing. A write (it rewrites the profile). */
    suspend fun cover(pages: List<String>): Result<LockState>

    /** Apply [draft] as the profile (a write: the device is clamped to it at once and the clamps reported). */
    suspend fun applyLimits(draft: LimitsDraft): Result<LimitsReport>

    /** Remove the profile (nothing on the device changes). */
    suspend fun clearLimits(): Result<Unit>

    suspend fun manifest(): ModeManifest?

    /**
     * `mode_resolve(platform, chosen)` for this app's platform class: the
     * manifest's class whose default is Simple (UI-MODES section 5: this app
     * opens in Simple). [chosen] "" asks for the default. Null with no core.
     */
    suspend fun resolve(chosen: String): ModeChoice?

    /** The core's notice for [e] ("Title: reason"), null when the core has none (or [e] is not the core's). */
    suspend fun noticeText(e: Throwable): String? = null

    /** The notice row [key], null when the core has no such row. */
    suspend fun notice(key: String): Notice? = null

    // -- WP P1EFIX -- (review p1e-a F7) one range table in the core, no copy here
    /** Every limits field's range (the core's `limits_field_ranges`), by field key; empty without a core. */
    suspend fun fieldRanges(): Map<String, LimitFieldRange> = emptyMap()

    /** S2: every output's limiter on at [ceilingDbfs], stereo pairs linked, one undo step. A write. */
    suspend fun protectAllOutputs(ceilingDbfs: Float = PROTECT_CEILING_DB): Result<Unit>

    /** S2's warning: the outputs (0-based) on, above [thresholdDb] gain, with no limiter. Never fails. */
    suspend fun outputsWithoutLimiter(thresholdDb: Float = UNPROTECTED_GAIN_DB): List<Int>

    /** `autoeq_preview`: what applying [profile] to wire channels [left] and [right] would do; writes nothing. */
    suspend fun autoeqPreview(profile: AutoEqProfile, left: Int, right: Int): Result<AutoEqPreview>

    /** `autoeq_apply`: a snapshot first, then one undo step; values held to the limits are reported. A write. */
    suspend fun autoeqApply(profile: AutoEqProfile, left: Int, right: Int): Result<AutoEqApply>

    companion object {
        /** S2's default ceiling. */
        const val PROTECT_CEILING_DB = -1f

        /** S2: "an output above -10 dB gain has no limiter". */
        const val UNPROTECTED_GAIN_DB = -10f

        /** The manifest's platform class whose default mode is Simple: this app's (UI-MODES section 5). */
        fun platformOf(m: ModeManifest): String =
            m.platforms.firstOrNull { p -> m.modes.any { it.id == SIMPLE && p in it.defaultOn } } ?: m.platforms.firstOrNull().orEmpty()

        const val SIMPLE = "simple"
        const val ADVANCED = "advanced"
        const val INSTALLER = "installer"
        const val LOCKED_USER = "locked_user"

        /** `LockState.heldBy` when the hub holds the PIN. */
        const val HELD_BY_HUB = "hub"
    }

    /** For bridges without a core (test doubles): no lock, no limits, every change refused. */
    object Unavailable : Installer {
        private fun <T> no(): Result<T> = Result.failure(InstallerException("Not available here."))

        override val lock: StateFlow<LockState?> = MutableStateFlow<LockState?>(null).asStateFlow()
        override val limits: StateFlow<LimitsStatus?> = MutableStateFlow<LimitsStatus?>(null).asStateFlow()
        override val profile: StateFlow<LimitsDraft?> = MutableStateFlow<LimitsDraft?>(null).asStateFlow()

        override suspend fun setFolder(dir: String): Result<Unit> = no()

        override suspend fun refresh(hub: Boolean) = Unit

        override suspend fun unlock(pin: String): Result<LockState> = no()

        override suspend fun lockNow(): Result<LockState> = no()

        override suspend fun setPin(pin: String): Result<String> = no()

        override suspend fun clearPin(): Result<LockState> = no()

        override suspend fun cover(pages: List<String>): Result<LockState> = no()

        override suspend fun applyLimits(draft: LimitsDraft): Result<LimitsReport> = no()

        override suspend fun clearLimits(): Result<Unit> = no()

        override suspend fun manifest(): ModeManifest? = null

        override suspend fun resolve(chosen: String): ModeChoice? = null

        override suspend fun protectAllOutputs(ceilingDbfs: Float): Result<Unit> = no()

        override suspend fun outputsWithoutLimiter(thresholdDb: Float): List<Int> = emptyList()

        override suspend fun autoeqPreview(profile: AutoEqProfile, left: Int, right: Int): Result<AutoEqPreview> = no()

        override suspend fun autoeqApply(profile: AutoEqProfile, left: Int, right: Int): Result<AutoEqApply> = no()
    }
}

internal class InstallerImpl(private val bridge: DspiBridgeImpl) : Installer {
    private val _lock = MutableStateFlow<LockState?>(null)
    override val lock: StateFlow<LockState?> = _lock.asStateFlow()
    private val _limits = MutableStateFlow<LimitsStatus?>(null)
    override val limits: StateFlow<LimitsStatus?> = _limits.asStateFlow()
    private val _profile = MutableStateFlow<LimitsDraft?>(null)
    override val profile: StateFlow<LimitsDraft?> = _profile.asStateFlow()

    private val core get() = bridge.core

    init {
        // A device opened or gone: its lock and its profile (no I/O but the hub's, read once per device)
        bridge.scope.launch(bridge.main) {
            bridge.deviceGeneration.collect { refresh(hub = true) }
        }
    }

    private data class Read(val lock: LockState?, val limits: LimitsStatus?, val profile: LimitsDraft?)

    override suspend fun refresh(hub: Boolean) {
        val r =
            bridge.onFfi {
                val lock = runCatching { if (hub) core.installerLockRefresh() else core.installerLockState() }
                    .recoverCatching { core.installerLockState() }
                    .getOrNull()
                Read(
                    lock,
                    runCatching { core.limitsStatus() }.getOrNull(),
                    runCatching { core.limitsProfileDocument() }.getOrNull()?.takeIf { it.isNotEmpty() }?.let(LimitsDraft::fromDocument)
                )
            }
        _lock.value = r.lock
        _limits.value = r.limits
        _profile.value = r.profile
    }

    /** Runs [block] on the FFI thread; a core error becomes a failure in the core's notice words. */
    private suspend fun <T> call(block: () -> T): Result<T> = bridge.onFfi {
        try {
            Result.success(block())
        } catch (e: DspiException) {
            Result.failure(InstallerException(wordsOf(e), e))
        }
    }

    /** FFI thread: the notice row for [e] as "Title: reason" (with the core's detail for a limit), or the plain words. */
    private fun wordsOf(e: DspiException): String = noticeOf(e) ?: Snapshots.wordsOf(e)

    private fun noticeOf(e: DspiException): String? = when (e) {
        is DspiException.InstallerLock, is DspiException.LimitExceeded, is DspiException.ConfirmationRequired,
        is DspiException.ImageRefused, is DspiException.LimitsUnknown, is DspiException.Denied, is DspiException.Locked,
        is DspiException.ReadOnly -> runCatching { core.noticeFor(e) }.getOrNull()?.let { n ->
            val detail = (e as? DspiException.LimitExceeded)?.msg?.trim().orEmpty()
            listOf("${n.title}: ${n.reason}", detail).filter { it.isNotEmpty() }.joinToString(" ")
        }
        else -> null
    }

    /** A write refused by G1 (read-only, a viewer, no device): the bridge reports it; nothing is sent. */
    private fun refused(name: String): Result<Nothing>? = if (bridge.writeRefused("installer.$name")) {
        Result.failure(InstallerException(Snapshots.wordsOf(WritesNotAllowedException(""))))
    } else {
        null
    }

    private fun locked(r: Result<LockState>): Result<LockState> {
        r.getOrNull()?.let { _lock.value = it }
        return r
    }

    override suspend fun setFolder(dir: String) = call { core.limitsSetDir(dir) }.also { refresh(hub = false) }

    override suspend fun unlock(pin: String) = locked(call { core.installerUnlock(pin) })

    override suspend fun lockNow() = locked(call { core.installerLock() })

    override suspend fun setPin(pin: String): Result<String> {
        val r = call { core.installerPinSet(pin) }
        r.getOrNull()?.let { _lock.value = it.state }
        return r.map { it.recoveryCode }
    }

    override suspend fun clearPin() = locked(call { core.installerPinClear() })

    override suspend fun cover(pages: List<String>): Result<LockState> {
        refused("cover")?.let { return it }
        return locked(call { core.installerLockCover(pages) }).also { refresh(hub = false) }
    }

    override suspend fun applyLimits(draft: LimitsDraft): Result<LimitsReport> {
        refused("applyLimits")?.let { return it }
        val serial = bridge.selectedSerial
        val r = call { core.limitsApplyProfile(draft.toDocument(serial)) }.map { LimitsReport(it.lines, it.held, it.notes) }
        bridge.refreshAll()
        refresh(hub = false)
        return r
    }

    override suspend fun clearLimits(): Result<Unit> = call { core.limitsClear() }.also { refresh(hub = false) }

    override suspend fun manifest(): ModeManifest? = bridge.onFfi { runCatching { core.modeManifest() }.getOrNull() }

    override suspend fun resolve(chosen: String): ModeChoice? = bridge.onFfi {
        runCatching { core.modeResolve(Installer.platformOf(core.modeManifest()), chosen) }.getOrNull()
    }

    override suspend fun noticeText(e: Throwable): String? = (e as? DspiException)?.let { d -> bridge.onFfi { noticeOf(d) } }

    override suspend fun notice(key: String): Notice? =
        bridge.onFfi { runCatching { core.noticeForKey(key, "") }.getOrNull()?.takeIf { it.key == key } }

    // -- WP P1EFIX --
    override suspend fun fieldRanges(): Map<String, LimitFieldRange> =
        bridge.onFfi { runCatching { core.limitsFieldRanges() }.getOrDefault(emptyList()).associateBy { it.key } }

    override suspend fun protectAllOutputs(ceilingDbfs: Float): Result<Unit> {
        refused("protectAllOutputs")?.let { return it }
        return call { core.protectAllOutputs(ceilingDbfs) }.also { bridge.refreshAll() }
    }

    override suspend fun outputsWithoutLimiter(thresholdDb: Float): List<Int> =
        bridge.onFfi { runCatching { core.outputsWithoutLimiter(thresholdDb).map { it } }.getOrDefault(emptyList()) }

    override suspend fun autoeqPreview(profile: AutoEqProfile, left: Int, right: Int) = call {
        core.autoeqPreview(profile, left, right, null)
    }

    override suspend fun autoeqApply(profile: AutoEqProfile, left: Int, right: Int): Result<AutoEqApply> {
        refused("autoeqApply")?.let { return it }
        return call { core.autoeqApply(profile, left, right, null) }.also { bridge.refreshAll() }
    }
}
