// One record being edited on a Control Surfaces page, and the app-scoped store
// that keeps it (WP AN9). The draft lives here, not in the page, so rotation (the
// activity is recreated) and leaving the page keep it; it belongs to one device
// serial. Every change goes through the core's rules (CG20) on the bridge's FFI
// thread; nothing reaches the device until Apply, and nothing is ever replayed.
package se.bofr.dspi.feature.settings.cs

import java.lang.ref.WeakReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator

/**
 * A record of [kind] in [slot]: the device's copy ([State.saved]), the draft, and
 * the core's description of the draft. Main thread only.
 */
class CsEditor internal constructor(
    private val bridge: DspiBridge,
    private val coordinator: SettingsSaveCoordinator,
    private val scope: CoroutineScope,
    val kind: String,
    val slot: Int,
    saved: String,
    savedName: String
) {
    data class State(
        val saved: String,
        val draft: String,
        val savedName: String,
        val name: String,
        val description: CsDescription? = null,
        /** The last refusal: an edit the rules did not take, or an Apply the device did not. */
        val error: String? = null,
        val working: Boolean = false
    ) {
        val dirty: Boolean get() = draft != saved || name != savedName
    }

    private val _state = MutableStateFlow(State(saved, saved, savedName, savedName))
    val state: StateFlow<State> = _state.asStateFlow()

    // Edits chain: each starts from the draft the previous one left
    private val lock = Mutex()

    /** Ask the core what the draft shows (again: the device's pins and groups may have moved). */
    fun describe(): Job = scope.launch { lock.withLock { describeLocked() } }

    private suspend fun describeLocked() {
        val r = CsRules.parseDescription(bridge.cs.describe(kind, slot, _state.value.draft))
        // A refusal keeps the last description readable (a lost connection, AN8 section 6)
        _state.update { s -> if (r.value != null) s.copy(description = r.value) else s.copy(error = r.error) }
    }

    /** One row's new value (an option's value, a number in natural units, or a boolean). */
    fun change(id: String, value: Any): Job = scope.launch {
        lock.withLock {
            _state.update { it.copy(working = true, error = null) }
            try {
                val r = CsRules.parseDraft(bridge.cs.edit(kind, slot, _state.value.draft, CsRules.change(id, value)))
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

    fun rename(name: String) {
        if (name.length <= NAME_MAX) _state.update { it.copy(name = name) }
    }

    /** Drop the draft: back to the device's record. */
    fun revert(): Job {
        _state.update { it.copy(draft = it.saved, name = it.savedName, error = null) }
        return describe()
    }

    /**
     * The device's record changed (an Apply, a revert, another client): an
     * untouched draft follows it; an edited one is kept.
     */
    fun sync(saved: String, savedName: String) {
        val s = _state.value
        if (saved == s.saved && savedName == s.savedName) return
        _state.update {
            if (it.dirty) {
                it.copy(saved = saved, savedName = savedName)
            } else {
                it.copy(saved = saved, draft = saved, savedName = savedName, name = savedName)
            }
        }
        describe()
    }

    /**
     * Send the draft and its name: live at once, and dirty until the save bar's
     * Save. A refusal keeps the draft and says why. [onDone] gets true when both took.
     */
    fun apply(onDone: (Boolean) -> Unit = {}): Job = scope.launch {
        lock.withLock {
            val s = _state.value
            if (!bridge.status.value.connected) {
                _state.update { it.copy(error = NOT_CONNECTED) }
                onDone(false)
                return@withLock
            }
            coordinator.beginCsOperation()
            _state.update { it.copy(working = true, error = null) }
            var error: String? = null
            try {
                if (s.draft != s.saved) {
                    // -- WP AN11 -- a display page or the display settings have their own operation
                    val op = if (kind == CsDisplay.KIND_PAGE || kind == CsDisplay.KIND_DISPLAY) {
                        CsDisplay.op(kind, slot, s.draft)
                    } else {
                        CsRules.bindingOp(slot, s.draft)
                    }
                    error =
                        if (op == null) "The draft is not a record" else CsRules.applyError(bridge.cs.apply(op), bridge.cs::statusMessage)
                }
                if (error == null && s.name != s.savedName) {
                    error = CsRules.applyError(bridge.cs.apply(CsModel.nameOp(slot, s.name.trim())), bridge.cs::statusMessage)
                }
            } finally {
                coordinator.endCsOperation()
            }
            _state.update {
                when {
                    error != null -> it.copy(error = error, working = false)
                    // Applied: the device's copy is the draft until the next read says otherwise
                    else -> it.copy(saved = s.draft, savedName = s.name, working = false)
                }
            }
            onDone(error == null)
        }
    }

    companion object {
        /** The firmware's name field (31 bytes and a terminator). */
        const val NAME_MAX = 31
        const val NOT_CONNECTED = "Not connected. The edit is kept; apply it when the device is back."
    }
}

/**
 * The open drafts of one device, app-scoped (E9) like [SettingsSaveCoordinator].
 * A different device's serial clears them; a re-plug of the same unit keeps them.
 */
class CsDrafts private constructor(private val bridge: DspiBridge) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val editors = mutableMapOf<String, CsEditor>()
    private var serial = ""

    /** The editor of one record, created from the device's copy, else brought up to date with it. */
    fun editor(kind: String, slot: Int, saved: String, savedName: String): CsEditor {
        val s = bridge.devices.value.selectedSerial
        // Only another device's serial clears: the first one seen (or a list not yet read) keeps what is open
        if (s.isNotEmpty() && s != serial) {
            if (serial.isNotEmpty()) editors.clear()
            serial = s
        }
        val key = "$kind/$slot"
        editors[key]?.let {
            it.sync(saved, savedName)
            return it
        }
        return CsEditor(bridge, SettingsSaveCoordinator.of(bridge), scope, kind, slot, saved, savedName).also {
            editors[key] = it
            it.describe()
        }
    }

    /** The record went away (removed): its draft with it. */
    fun drop(kind: String, slot: Int) {
        editors.remove("$kind/$slot")
    }

    /**
     * Add: a fresh binding of [component] in the free [slot], applied at once with
     * the core's defaults (AN8 section 2). [onDone] gets null when it took, else why not.
     */
    fun add(slot: Int, component: Int, onDone: (String?) -> Unit): Job = scope.launch {
        val coordinator = SettingsSaveCoordinator.of(bridge)
        coordinator.beginCsOperation()
        val error = try {
            val d = CsRules.parseDraft(bridge.cs.newBinding(slot, component))
            val op = d.value?.let { CsRules.bindingOp(slot, it) }
            if (op ==
                null
            ) {
                d.error ?: "The new control could not be made"
            } else {
                CsRules.applyError(bridge.cs.apply(op), bridge.cs::statusMessage)
            }
        } finally {
            coordinator.endCsOperation()
        }
        if (error == null) drop(CsRules.KIND_BINDING, slot)
        onDone(error)
    }

    /** True while a record of [kind] in [slot] has edits not yet applied. */
    fun isDirty(kind: String, slot: Int): Boolean = editors["$kind/$slot"]?.state?.value?.dirty == true

    companion object {
        private var cached: Pair<WeakReference<DspiBridge>, CsDrafts>? = null

        fun of(bridge: DspiBridge): CsDrafts {
            cached?.let { (ref, d) -> if (ref.get() === bridge) return d }
            return CsDrafts(bridge).also { cached = WeakReference(bridge) to it }
        }
    }
}
