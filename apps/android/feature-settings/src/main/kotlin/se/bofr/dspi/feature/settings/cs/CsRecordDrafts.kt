// The drafts of the Channel Groups and Macros pages (AN8's S9, S11, S12) and the
// app-scoped store that keeps them, as CsDrafts keeps the controls' (WP AN12).
// A group is one record of CG20's kind "group"; a macro is a name and a list of
// records of kind "step", which the core describes and edits one at a time. Every
// change goes through the core's rules on the bridge's FFI thread; nothing reaches
// the device until Apply, and nothing is ever replayed.
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
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.long
import se.bofr.dspi.ui.json.str

/** The snapshot's group and macro records as drafts, and the operations that send them (WP AN12). */
object CsRecords {
    const val KIND_GROUP = "group"
    const val KIND_STEP = "step"
    const val KIND_MACRO = "macro"

    /** The firmware's group and macro name fields (32 bytes with the terminator). */
    const val NAME_MAX = 31

    /** A group's rules draft: the record without its name, which the page edits apart. */
    fun groupDraft(targetKind: Int, memberMask: Long): String =
        Json.write(linkedMapOf("targetKind" to targetKind, "memberMask" to memberMask, "name" to ""))

    /** An empty group: the all-zero record that clears the slot. */
    val GROUP_CLEARED: String = groupDraft(0, 0)

    /** A new group's draft (the Mac's `addGroup`): outputs, no members yet, so not appliable until one is picked. */
    val GROUP_NEW: String = groupDraft(OUTPUTS, 0)
    private const val OUTPUTS = 2

    /** Group [g] of the `cs_snapshot` JSON as a draft and its name; null when the device has no such group. */
    fun group(model: String, g: Int): Pair<String, String>? {
        val m = Json.parseObject(model)?.list("groups")?.getOrNull(g) as? Map<*, *> ?: return null

        @Suppress("UNCHECKED_CAST")
        val x = m as Map<String, Any?>
        return groupDraft(x.int("targetKind") ?: 0, x.long("memberMask") ?: 0) to (x.str("name") ?: "")
    }

    /**
     * Macro [m] of the `cs_snapshot` JSON, its steps whole (flags and step size
     * included, which `CsModel` does not keep); null when the device has no such macro.
     */
    fun macro(model: String, m: Int): CsMacroDraft? {
        val x = Json.parseObject(model)?.list("macros")?.getOrNull(m) as? Map<*, *> ?: return null

        @Suppress("UNCHECKED_CAST")
        val mc = x as Map<String, Any?>
        val steps = mc.list("steps").orEmpty().mapNotNull { (it as? Map<*, *>)?.let(Json::write) }
        return CsMacroDraft(mc.str("name") ?: "", steps)
    }

    /** How many steps a macro holds on this device (`maxSteps`); at least 1. */
    fun maxSteps(model: String): Int = (Json.parseObject(model)?.int("maxSteps") ?: 0).coerceAtLeast(1)

    /** `cs.apply` for group [g]: the draft with [name]; null for a draft that is not an object. */
    fun groupOp(g: Int, draft: String, name: String): String? {
        val d = Json.parseObject(draft)?.toMutableMap() ?: return null
        d["name"] = name
        return Json.write(linkedMapOf("op" to "group", "index" to g, "group" to d))
    }

    /** `cs.apply` for macro [m]; null when a step is not an object. */
    fun macroOp(m: Int, draft: CsMacroDraft): String? {
        val steps = draft.steps.map { Json.parseObject(it) ?: return null }
        return Json.write(
            linkedMapOf("op" to "macro", "index" to m, "macro" to linkedMapOf("name" to draft.name, "steps" to steps))
        )
    }

    /** An empty macro (no name, no steps) frees the slot. */
    fun macroClearOp(m: Int): String = Json.write(
        linkedMapOf("op" to "macro", "index" to m, "macro" to linkedMapOf("name" to "", "steps" to emptyList<Any>()))
    )

    fun macroCancelOp(): String = Json.write(linkedMapOf("op" to "macroCancel"))

    /** An aux output's live level in percent: instant, never a flash write. */
    fun auxLevelOp(slot: Int, percent: Double): String = Json.write(linkedMapOf("op" to "auxLevel", "index" to slot, "percent" to percent))

    /** The snapshot's live aux level of [slot] (8.8) in percent. */
    fun auxPercent(level: Int): Double = (level.coerceAtLeast(0) / 256.0).coerceIn(0.0, 100.0)

    /** Two drafts are the same record whatever order their keys came in (the core's or ours). */
    fun sameRecord(a: String, b: String): Boolean = a == b || (Json.parseObject(a)?.let { it == Json.parseObject(b) } ?: false)

    /** A step moved by [by] places in [list]; the list unchanged when it would leave it. */
    fun <T> moved(list: List<T>, from: Int, by: Int): List<T> {
        val to = from + by
        if (from !in list.indices || to !in list.indices) return list
        return list.toMutableList().also { it.add(to, it.removeAt(from)) }
    }
}

/** A macro as the page edits it: its name and its steps' `cs_snapshot` records as JSON text. */
data class CsMacroDraft(val name: String, val steps: List<String>) {
    fun sameAs(o: CsMacroDraft): Boolean =
        name == o.name && steps.size == o.steps.size && steps.indices.all { CsRecords.sameRecord(steps[it], o.steps[it]) }
}

/**
 * Channel group [slot]: the device's copy, the draft, and the core's description
 * of the draft. The name is kept apart from the rules' draft, as a control's is.
 * Main thread only.
 */
class CsGroupEditor internal constructor(
    private val bridge: DspiBridge,
    private val coordinator: SettingsSaveCoordinator,
    private val scope: CoroutineScope,
    val slot: Int,
    saved: String,
    savedName: String,
    draft: String = saved,
    name: String = savedName
) {
    data class State(
        val saved: String,
        val draft: String,
        val savedName: String,
        val name: String,
        val description: CsDescription? = null,
        val error: String? = null,
        val working: Boolean = false
    ) {
        val dirty: Boolean get() = name != savedName || !CsRecords.sameRecord(draft, saved)

        /** The device has this group (Remove then clears it there). */
        val stored: Boolean get() = !CsRecords.sameRecord(saved, CsRecords.GROUP_CLEARED)

        /** The core's "noMembers" note: the firmware refuses a group with no members. */
        val appliable: Boolean get() = description != null && description.rows.none { it.id == "noMembers" }
    }

    private val _state = MutableStateFlow(State(saved, draft, savedName, name))
    val state: StateFlow<State> = _state.asStateFlow()
    private val lock = Mutex()

    fun describe(): Job = scope.launch { lock.withLock { describeLocked() } }

    private suspend fun describeLocked() {
        val r = CsRules.parseDescription(bridge.cs.describe(CsRecords.KIND_GROUP, slot, _state.value.draft))
        _state.update { s -> if (r.value != null) s.copy(description = r.value) else s.copy(error = r.error) }
    }

    /** One row's new value: the channel type, or a member's toggle. */
    fun change(id: String, value: Any): Job = scope.launch {
        lock.withLock {
            _state.update { it.copy(working = true, error = null) }
            try {
                val r = CsRules.parseDraft(bridge.cs.edit(CsRecords.KIND_GROUP, slot, _state.value.draft, CsRules.change(id, value)))
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
        if (name.length <= CsRecords.NAME_MAX) _state.update { it.copy(name = name) }
    }

    fun revert(): Job {
        _state.update { it.copy(draft = it.saved, name = it.savedName, error = null) }
        return describe()
    }

    /** The device's group changed: an untouched draft follows it; an edited one is kept. */
    fun sync(saved: String, savedName: String) {
        val s = _state.value
        if (CsRecords.sameRecord(saved, s.saved) && savedName == s.savedName) return
        _state.update {
            if (it.dirty) {
                it.copy(
                    saved = saved,
                    savedName = savedName
                )
            } else {
                it.copy(saved = saved, draft = saved, savedName = savedName, name = savedName)
            }
        }
        describe()
    }

    /** Send the draft with its name: live at once, dirty until Save. A refusal keeps the draft and says why. */
    fun apply(onDone: (Boolean) -> Unit = {}): Job = scope.launch {
        lock.withLock {
            val s = _state.value
            if (!bridge.status.value.connected) {
                _state.update { it.copy(error = CsEditor.NOT_CONNECTED) }
                onDone(false)
                return@withLock
            }
            coordinator.beginCsOperation()
            _state.update { it.copy(working = true, error = null) }
            val error = try {
                val op = CsRecords.groupOp(slot, s.draft, s.name.trim())
                if (op == null) "The draft is not a record" else CsRules.applyError(bridge.cs.apply(op), bridge.cs::statusMessage)
            } finally {
                coordinator.endCsOperation()
            }
            _state.update {
                if (error !=
                    null
                ) {
                    it.copy(error = error, working = false)
                } else {
                    it.copy(saved = s.draft, savedName = s.name.trim(), working = false)
                }
            }
            onDone(error == null)
        }
    }
}

/**
 * Macro [slot]: the device's copy, the draft (name and steps), and the core's
 * description of each step. Steps are reordered and removed here (the order is
 * the record's, not a rule); what one step offers is the core's. Main thread only.
 */
class CsMacroEditor internal constructor(
    private val bridge: DspiBridge,
    private val coordinator: SettingsSaveCoordinator,
    private val scope: CoroutineScope,
    val slot: Int,
    saved: CsMacroDraft,
    draft: CsMacroDraft = saved,
    val maxSteps: Int
) {
    data class State(
        val saved: CsMacroDraft,
        val draft: CsMacroDraft,
        /** The core's description of each draft step, in the draft's order; null until read. */
        val steps: List<CsDescription?> = emptyList(),
        val error: String? = null,
        val working: Boolean = false
    ) {
        val dirty: Boolean get() = !draft.sameAs(saved)

        val stored: Boolean get() = saved.name.isNotEmpty() || saved.steps.isNotEmpty()
    }

    private val _state = MutableStateFlow(State(saved, draft))
    val state: StateFlow<State> = _state.asStateFlow()
    private val lock = Mutex()

    /** Describe every step again (the device's channels and groups may have moved). */
    fun describe(): Job = scope.launch { lock.withLock { describeAllLocked() } }

    private suspend fun describeAllLocked() {
        val steps = _state.value.draft.steps
        var error: String? = null
        val out = steps.map { s ->
            val r = CsRules.parseDescription(bridge.cs.describe(CsRecords.KIND_STEP, 0, s))
            if (r.value == null) error = r.error
            r.value
        }
        // Only if the steps did not move meanwhile (the lock keeps edits out; a rename does not touch them)
        _state.update { if (it.draft.steps == steps) it.copy(steps = out, error = error ?: it.error) else it }
    }

    private fun setDraft(d: CsMacroDraft, descriptions: List<CsDescription?>) {
        _state.update { it.copy(draft = d, steps = descriptions, error = null) }
    }

    fun rename(name: String) {
        if (name.length <= CsRecords.NAME_MAX) _state.update { it.copy(draft = it.draft.copy(name = name)) }
    }

    /** A fresh step from the core, at the end; [onAdded] gets its index. */
    fun addStep(onAdded: (Int) -> Unit = {}): Job = scope.launch {
        lock.withLock {
            if (_state.value.draft.steps.size >= maxSteps) return@withLock
            _state.update { it.copy(working = true, error = null) }
            try {
                val r = CsRules.parseDraft(bridge.cs.newRecord(CsRecords.KIND_STEP))
                val step = r.value
                if (step == null) {
                    _state.update { it.copy(error = r.error) }
                    return@withLock
                }
                val d = CsRules.parseDescription(bridge.cs.describe(CsRecords.KIND_STEP, 0, step)).value
                val s = _state.value
                setDraft(s.draft.copy(steps = s.draft.steps + step), padded(s).plus(d))
                onAdded(_state.value.draft.steps.size - 1)
            } finally {
                _state.update { it.copy(working = false) }
            }
        }
    }

    private fun padded(s: State): List<CsDescription?> = s.draft.steps.indices.map { s.steps.getOrNull(it) }

    fun moveStep(index: Int, by: Int) {
        val s = _state.value
        setDraft(s.draft.copy(steps = CsRecords.moved(s.draft.steps, index, by)), CsRecords.moved(padded(s), index, by))
    }

    /** Remove a step and close the gap (the sequencer runs the first `stepCount`). */
    fun removeStep(index: Int) {
        val s = _state.value
        if (index !in s.draft.steps.indices) return
        setDraft(
            s.draft.copy(steps = s.draft.steps.filterIndexed { i, _ -> i != index }),
            padded(s).filterIndexed { i, _ -> i != index }
        )
    }

    /** One row of step [index] changed: the core makes the next step. */
    fun changeStep(index: Int, id: String, value: Any): Job = scope.launch {
        lock.withLock {
            val step = _state.value.draft.steps.getOrNull(index) ?: return@withLock
            _state.update { it.copy(working = true, error = null) }
            try {
                val r = CsRules.parseDraft(bridge.cs.edit(CsRecords.KIND_STEP, 0, step, CsRules.change(id, value)))
                val next = r.value
                if (next == null) {
                    _state.update { it.copy(error = r.error) }
                    return@withLock
                }
                val d = CsRules.parseDescription(bridge.cs.describe(CsRecords.KIND_STEP, 0, next))
                _state.update { s ->
                    if (s.draft.steps.getOrNull(index) != step) return@update s
                    val steps = s.draft.steps.toMutableList().also { it[index] = next }
                    val descs = padded(s).toMutableList().also { it[index] = d.value ?: it[index] }
                    s.copy(draft = s.draft.copy(steps = steps), steps = descs, error = d.error)
                }
            } finally {
                _state.update { it.copy(working = false) }
            }
        }
    }

    fun revert(): Job {
        _state.update { it.copy(draft = it.saved, error = null) }
        return describe()
    }

    /** The device's macro changed: an untouched draft follows it; an edited one is kept. */
    fun sync(saved: CsMacroDraft) {
        val s = _state.value
        if (saved.sameAs(s.saved)) return
        _state.update { if (it.dirty) it.copy(saved = saved) else it.copy(saved = saved, draft = saved) }
        describe()
    }

    /** Send the whole macro: live at once, dirty until Save. A refusal keeps the draft and says why. */
    fun apply(onDone: (Boolean) -> Unit = {}): Job = scope.launch {
        lock.withLock {
            val s = _state.value
            if (!bridge.status.value.connected) {
                _state.update { it.copy(error = CsEditor.NOT_CONNECTED) }
                onDone(false)
                return@withLock
            }
            coordinator.beginCsOperation()
            _state.update { it.copy(working = true, error = null) }
            val d = s.draft.copy(name = s.draft.name.trim())
            val error = try {
                val op = CsRecords.macroOp(slot, d)
                if (op == null) "The draft is not a record" else CsRules.applyError(bridge.cs.apply(op), bridge.cs::statusMessage)
            } finally {
                coordinator.endCsOperation()
            }
            _state.update {
                if (error != null) it.copy(error = error, working = false) else it.copy(saved = d, draft = d, working = false)
            }
            onDone(error == null)
        }
    }
}

/**
 * The open group and macro drafts of one device, app-scoped (E9) like
 * [CsDrafts]. A different device's serial clears them; a re-plug of the same
 * unit keeps them.
 */
class CsRecordDrafts private constructor(private val bridge: DspiBridge) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val groups = mutableMapOf<Int, CsGroupEditor>()
    private val macros = mutableMapOf<Int, CsMacroEditor>()
    private var serial = ""

    private fun checkSerial() {
        val s = bridge.devices.value.selectedSerial
        if (s.isNotEmpty() && s != serial) {
            if (serial.isNotEmpty()) {
                groups.clear()
                macros.clear()
            }
            serial = s
        }
    }

    /** Group [g]'s editor, from the device's copy, else brought up to date with it. */
    fun group(g: Int, saved: String, savedName: String): CsGroupEditor {
        checkSerial()
        groups[g]?.let {
            it.sync(saved, savedName)
            return it
        }
        return CsGroupEditor(bridge, SettingsSaveCoordinator.of(bridge), scope, g, saved, savedName).also {
            groups[g] = it
            it.describe()
        }
    }

    /** Add Group: a new draft in the free group [g], not applied until it has a member (the Mac's `addGroup`). */
    fun newGroup(g: Int): CsGroupEditor {
        checkSerial()
        return CsGroupEditor(
            bridge,
            SettingsSaveCoordinator.of(bridge),
            scope,
            g,
            CsRecords.GROUP_CLEARED,
            "",
            CsRecords.GROUP_NEW,
            "Group ${g + 1}"
        ).also {
            groups[g] = it
            it.describe()
        }
    }

    /** Macro [m]'s editor, from the device's copy, else brought up to date with it. */
    fun macro(m: Int, saved: CsMacroDraft, maxSteps: Int): CsMacroEditor {
        checkSerial()
        macros[m]?.let {
            it.sync(saved)
            return it
        }
        return CsMacroEditor(bridge, SettingsSaveCoordinator.of(bridge), scope, m, saved, maxSteps = maxSteps).also {
            macros[m] = it
            it.describe()
        }
    }

    /** Add Macro: a named draft with no steps in the free macro [m], not applied yet (the Mac's `addMacro`). */
    fun newMacro(m: Int, maxSteps: Int): CsMacroEditor {
        checkSerial()
        return CsMacroEditor(
            bridge,
            SettingsSaveCoordinator.of(bridge),
            scope,
            m,
            CsMacroDraft("", emptyList()),
            CsMacroDraft("Macro ${m + 1}", emptyList()),
            maxSteps
        ).also { macros[m] = it }
    }

    fun dropGroup(g: Int) {
        groups.remove(g)
    }

    fun dropMacro(m: Int) {
        macros.remove(m)
    }

    /** Group [g]'s open editor, if any, as it is (no sync). */
    internal fun openGroup(g: Int): CsGroupEditor? = groups[g]

    internal fun openMacro(m: Int): CsMacroEditor? = macros[m]

    /** True while group [g] has edits not yet applied (a new group's included). */
    fun isGroupDirty(g: Int): Boolean = groups[g]?.state?.value?.dirty == true

    fun isMacroDirty(m: Int): Boolean = macros[m]?.state?.value?.dirty == true

    companion object {
        private var cached: Pair<WeakReference<DspiBridge>, CsRecordDrafts>? = null

        fun of(bridge: DspiBridge): CsRecordDrafts {
            cached?.let { (ref, d) -> if (ref.get() === bridge) return d }
            return CsRecordDrafts(bridge).also { cached = WeakReference(bridge) to it }
        }
    }
}
