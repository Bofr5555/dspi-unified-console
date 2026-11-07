// The IR receiver's remote buttons (AN8's S5 and D2, WP AN10): one button being
// edited, and the learn that gives it its code. As CsEditor, the draft is the
// snapshot's `ir` record, every row change goes through the core's rules (CG20,
// kind `ir`) and nothing reaches the device until Apply. The code itself comes
// from a learn: the receiver takes the next press within the firmware's 10 s
// window (control_surfaces_ir.c), which the page polls a little past, as the Mac.
package se.bofr.dspi.feature.settings.cs

import java.lang.ref.WeakReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.bool
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.long
import se.bofr.dspi.ui.json.str

/** A learn's answer (`learnRead`): `CS_IR_LEARN_*` [state], and the code once done. */
data class CsLearnResult(val state: Int, val protocol: Int, val code: Long)

object CsIr {
    const val KIND = "ir"

    const val LEARN_ARMED = 1
    const val LEARN_DONE = 2
    const val LEARN_TIMEOUT = 3

    /** The firmware's learn window (`IR_LEARN_WINDOW_US`). */
    const val LEARN_WINDOW_MS = 10_000L
    const val POLL_MS = 100L

    /** How long the page polls: the window and a margin, as the Mac (115 x 100 ms). */
    const val POLL_LIMIT_MS = 11_500L

    // The Console's words (DSPi_ConsoleApp.swift, dspi-tui surfaces.rs)
    const val LEARN_PROMPT = "Point the remote at the receiver and press the button to learn."
    const val LEARN_WAITING = "Waiting for a button..."
    const val LEARN_TIMED_OUT = "No remote button was detected - try again."
    const val LEARN_STOPPED = "Learn stopped."
    const val LEARN_CANCELLED = "Learn cancelled."
    const val NOT_LIVE = "Apply the receiver above before learning remote buttons."
    const val NOT_LEARNED = "Not learned"
    const val LEARN_FIRST = "Learn the button before applying it."

    private const val UNREADABLE = "the core's answer could not be read"

    /** `learnArm`, `learnRead`, `learnCancel` for `cs.apply`. */
    val ARM: String = Json.write(mapOf("op" to "learnArm"))
    val READ: String = Json.write(mapOf("op" to "learnRead"))
    val CANCEL: String = Json.write(mapOf("op" to "learnCancel"))

    /**
     * The core's `ir_protocol_name`, which no call answers yet: words for the
     * code chip only, never a rule.
     */
    fun protocolName(p: Int): String = when (p) {
        1 -> "NEC"
        2 -> "RC5"
        3 -> "RC6"
        4 -> "Generic"
        else -> "None"
    }

    fun learnedText(protocol: Int): String = "Learned a ${protocolName(protocol)} code. Apply to keep it."

    /** The snapshot's remote buttons, each as its record's JSON. */
    fun records(snapshot: String): List<String> = Json.parseObject(snapshot)?.list("ir").orEmpty().map { Json.write(it) }

    /** A remote button has a code once learned; protocol 0 is an empty entry. */
    fun isLearned(record: String): Boolean = (Json.parseObject(record)?.int("protocol") ?: 0) != 0

    /** "NEC 0x20DF40BF", or [NOT_LEARNED]. */
    fun chip(record: String): String {
        val m = Json.parseObject(record) ?: return NOT_LEARNED
        val p = m.int("protocol") ?: 0
        if (p == 0) return NOT_LEARNED
        return "${protocolName(p)} 0x%08X".format(m.long("code") ?: 0L)
    }

    /** The draft with a learned code; the rest (what it drives) stays. */
    fun withCode(draft: String, protocol: Int, code: Long): String? {
        val m = Json.parseObject(draft) ?: return null
        return Json.write(
            LinkedHashMap(m).apply {
                put("protocol", protocol)
                put("code", code)
            }
        )
    }

    /** The first entry with no code and no open draft ([drafted]), or null when there is none. */
    fun firstFree(records: List<String>, drafted: Set<Int> = emptySet()): Int? =
        records.indices.firstOrNull { !isLearned(records[it]) && it !in drafted }

    /** `cs.apply` for one remote button; null for a draft that is not an object. */
    fun op(sub: Int, draft: String): String? {
        val d = Json.parseObject(draft) ?: return null
        return Json.write(linkedMapOf("op" to KIND, "index" to sub, "command" to d))
    }

    /** Removing a remote button: an empty entry (every field 0). */
    fun clearOp(sub: Int): String = Json.write(linkedMapOf("op" to KIND, "index" to sub, "command" to emptyMap<String, Any?>()))

    /** `learnRead`'s answer, or the core's refusal. */
    fun parseLearn(json: String, statusText: (Int) -> String): CsRuleReply<CsLearnResult> {
        val m = Json.parseObject(json) ?: return CsRuleReply(null, UNREADABLE)
        if (m.bool("ok") != true) return CsRuleReply(null, CsRules.applyError(json, statusText) ?: m.str("error") ?: UNREADABLE)
        return CsRuleReply(CsLearnResult(m.int("state") ?: 0, m.int("protocol") ?: 0, m.long("code") ?: 0L), null)
    }

    /** What the page says when a learn ended with [r] (null: still armed past the window). */
    fun outcome(r: CsLearnResult?): Pair<String, Boolean> = when {
        r == null || r.state == LEARN_TIMEOUT -> LEARN_TIMED_OUT to true
        r.state == LEARN_DONE && r.code != 0L -> learnedText(r.protocol) to false
        else -> LEARN_STOPPED to false
    }
}

/** One remote button ([sub]): the device's copy, the draft, and the core's description of it. Main thread only. */
class CsIrEditor internal constructor(
    private val bridge: DspiBridge,
    private val coordinator: SettingsSaveCoordinator,
    private val scope: CoroutineScope,
    val sub: Int,
    saved: String,
    draft: String
) {
    data class State(
        val saved: String,
        val draft: String,
        val description: CsDescription? = null,
        /** The last refusal: an edit the rules did not take, or an Apply the device did not. */
        val error: String? = null,
        /** A learn's outcome, and whether it is a warning. */
        val message: Pair<String, Boolean>? = null,
        val working: Boolean = false
    ) {
        val dirty: Boolean get() = draft != saved
        val learned: Boolean get() = CsIr.isLearned(draft)
    }

    private val _state = MutableStateFlow(State(saved, draft))
    val state: StateFlow<State> = _state.asStateFlow()
    private val lock = Mutex()

    fun describe(): Job = scope.launch { lock.withLock { describeLocked() } }

    private suspend fun describeLocked() {
        val r = CsRules.parseDescription(bridge.cs.describe(CsIr.KIND, sub, _state.value.draft))
        _state.update { s -> if (r.value != null) s.copy(description = r.value) else s.copy(error = r.error) }
    }

    /** One row's new value, through the core's rules. */
    fun change(id: String, value: Any): Job = scope.launch {
        lock.withLock {
            _state.update { it.copy(working = true, error = null) }
            try {
                val r = CsRules.parseDraft(bridge.cs.edit(CsIr.KIND, sub, _state.value.draft, CsRules.change(id, value)))
                if (r.value != null) {
                    _state.update { it.copy(draft = r.value) }
                    describeLocked()
                } else {
                    _state.update { it.copy(error = r.error) }
                }
            } finally {
                _state.update { it.copy(working = false) }
            }
        }
    }

    /** A learn's code goes into the draft; Apply sends it. */
    internal fun learned(r: CsLearnResult): Job = scope.launch {
        lock.withLock {
            CsIr.withCode(_state.value.draft, r.protocol, r.code)?.let { d -> _state.update { it.copy(draft = d) } }
            describeLocked()
        }
    }

    internal fun say(message: Pair<String, Boolean>?) = _state.update { it.copy(message = message) }

    fun revert(): Job {
        _state.update { it.copy(draft = it.saved, error = null, message = null) }
        return describe()
    }

    /** The device's record changed: an untouched draft follows it; an edited one is kept. */
    fun sync(saved: String) {
        val s = _state.value
        if (saved == s.saved) return
        _state.update { if (it.dirty) it.copy(saved = saved) else it.copy(saved = saved, draft = saved) }
        describe()
    }

    /** Send the draft, live at once and unsaved until Save. A refusal keeps the draft and says why. */
    fun apply(onDone: (Boolean) -> Unit = {}): Job = scope.launch {
        lock.withLock {
            val s = _state.value
            val refusal = when {
                !bridge.status.value.connected -> CsEditor.NOT_CONNECTED
                !s.learned -> CsIr.LEARN_FIRST
                else -> null
            }
            if (refusal != null) {
                _state.update { it.copy(error = refusal) }
                onDone(false)
                return@withLock
            }
            coordinator.beginCsOperation()
            _state.update { it.copy(working = true, error = null) }
            val error = try {
                val op = CsIr.op(sub, s.draft)
                if (op == null) "The draft is not a record" else CsRules.applyError(bridge.cs.apply(op), bridge.cs::statusMessage)
            } finally {
                coordinator.endCsOperation()
            }
            _state.update {
                if (error != null) it.copy(error = error, working = false) else it.copy(saved = s.draft, working = false, message = null)
            }
            onDone(error == null)
        }
    }
}

/**
 * The remote buttons' drafts and the one learn of a device, app-scoped like
 * [CsDrafts], so rotation and leaving the page keep them. A different device's
 * serial clears them.
 */
class CsIrStore private constructor(private val bridge: DspiBridge) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val editors = mutableMapOf<Int, CsIrEditor>()
    private var serial = ""

    /** The learn in progress: its button, the time left in the window, and which learn it is. */
    data class Learn(val sub: Int, val msLeft: Long, internal val id: Int)

    private val _learn = MutableStateFlow<Learn?>(null)
    val learn: StateFlow<Learn?> = _learn.asStateFlow()
    private var learns = 0

    /** The time seam between polls; tests replace it. */
    internal var pause: suspend (Long) -> Unit = { delay(it) }

    private fun coordinator() = SettingsSaveCoordinator.of(bridge)

    private fun checkSerial() {
        val s = bridge.devices.value.selectedSerial
        if (s.isNotEmpty() && s != serial) {
            if (serial.isNotEmpty()) editors.clear()
            serial = s
        }
    }

    /** The editor of remote button [sub], created from the device's copy, else brought up to date with it. */
    fun editor(sub: Int, saved: String): CsIrEditor {
        checkSerial()
        editors[sub]?.let {
            it.sync(saved)
            return it
        }
        return CsIrEditor(bridge, coordinator(), scope, sub, saved, saved).also {
            editors[sub] = it
            it.describe()
        }
    }

    /**
     * Add: a fresh remote button from the core in the empty entry [sub], kept as a
     * draft (it has no code until a learn). [onDone] gets null when it is ready, else why not.
     */
    fun add(sub: Int, saved: String, onDone: (String?) -> Unit): Job = scope.launch {
        checkSerial()
        val d = CsRules.parseDraft(bridge.cs.newRecord(CsIr.KIND))
        if (d.value == null) {
            onDone(d.error)
            return@launch
        }
        editors[sub] = CsIrEditor(bridge, coordinator(), scope, sub, saved, d.value).also { it.describe() }
        onDone(null)
    }

    fun drop(sub: Int) {
        editors.remove(sub)
    }

    fun isDirty(sub: Int): Boolean = editors[sub]?.state?.value?.dirty == true

    /** The buttons with an open, unapplied draft (a fresh one not yet learned among them). */
    fun dirtySubs(): Set<Int> = editors.filterValues { it.state.value.dirty }.keys

    /** The device's buttons moved: untouched drafts follow. */
    fun sync(records: List<String>) {
        editors.forEach { (sub, e) -> records.getOrNull(sub)?.let(e::sync) }
    }

    /**
     * D2: arm the receiver and wait for one press, then put its code in [e]'s
     * draft. One learn at a time; it holds the page's busy flag while it listens.
     */
    fun startLearn(e: CsIrEditor): Job? {
        if (_learn.value != null) return null
        val id = ++learns
        _learn.value = Learn(e.sub, CsIr.LEARN_WINDOW_MS, id)
        e.say(null)
        val coordinator = coordinator()
        return scope.launch {
            coordinator.beginCsOperation()
            try {
                val armed = CsRules.applyError(bridge.cs.apply(CsIr.ARM), bridge.cs::statusMessage)
                if (armed != null) {
                    if (ours(id)) e.say(armed to true)
                    return@launch
                }
                var waited = 0L
                var result: CsLearnResult? = null
                while (waited < CsIr.POLL_LIMIT_MS && ours(id)) {
                    pause(CsIr.POLL_MS)
                    waited += CsIr.POLL_MS
                    if (!ours(id)) break
                    _learn.value = Learn(e.sub, (CsIr.LEARN_WINDOW_MS - waited).coerceAtLeast(0), id)
                    val r = CsIr.parseLearn(bridge.cs.apply(CsIr.READ), bridge.cs::statusMessage)
                    if (r.value == null) {
                        // A lost connection or a refused read: stop listening and say why
                        if (ours(id)) e.say((r.error ?: CsIr.LEARN_STOPPED) to true)
                        bridge.cs.apply(CsIr.CANCEL)
                        return@launch
                    }
                    if (r.value.state != CsIr.LEARN_ARMED) {
                        result = r.value
                        break
                    }
                }
                // Cancelled: cancelLearn said so and disarmed
                if (!ours(id)) return@launch
                // Still armed past the window: disarm, so a later press is not taken
                if (result == null) bridge.cs.apply(CsIr.CANCEL)
                if (result != null && result.state == CsIr.LEARN_DONE && result.code != 0L) e.learned(result)
                e.say(CsIr.outcome(result))
            } finally {
                coordinator.endCsOperation()
                if (ours(id)) _learn.value = null
            }
        }
    }

    private fun ours(id: Int) = _learn.value?.id == id

    /** Stop listening: the page says so at once; the receiver is disarmed after. */
    fun cancelLearn(): Job? {
        val l = _learn.value ?: return null
        _learn.value = null
        editors[l.sub]?.say(CsIr.LEARN_CANCELLED to false)
        return scope.launch { bridge.cs.apply(CsIr.CANCEL) }
    }

    companion object {
        private var cached: Pair<WeakReference<DspiBridge>, CsIrStore>? = null

        fun of(bridge: DspiBridge): CsIrStore {
            cached?.let { (ref, s) -> if (ref.get() === bridge) return s }
            return CsIrStore(bridge).also { cached = WeakReference(bridge) to it }
        }
    }
}
