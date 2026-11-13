// -- WP LKUI1 -- the System screen's state: the hub's audio link installation as
// the core gives it (DspiBridge.linkSystem*, dspi-core link/system.rs), the last
// recall, and the Clock-and-links draft with the revision it was made on.
package se.bofr.dspi.feature.home.system

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkRoomLipSync
import se.bofr.dspi.core.LinkSceneRecall
import se.bofr.dspi.core.LinkSystemEdits
import se.bofr.dspi.core.LinkSystemView
import se.bofr.dspi.core.LinkUnitLatency

/** What the last recall did, in the core's words (or the hub's, for a refusal by role). */
data class RecallLine(val name: String, val taken: Boolean, val message: String, val writes: List<String>)

/**
 * The Clock-and-links edits not saved yet, made on [revision]. [rooms] and
 * [units] map an id to µs, or to null for "the model's own".
 */
data class SystemDraft(
    val revision: Long = 0,
    val delaySet: Boolean = false,
    val delayAuto: Boolean = true,
    val delayFrames: Long = 0,
    val rooms: Map<String, Long?> = emptyMap(),
    val units: Map<String, Long?> = emptyMap()
) {
    val dirty: Boolean get() = delaySet || rooms.isNotEmpty() || units.isNotEmpty()

    fun edits() = LinkSystemEdits(
        setSystemDelay = delaySet,
        systemDelayFrames = if (delayAuto) null else delayFrames,
        rooms = rooms.map { (id, us) -> LinkRoomLipSync(id, us) },
        units = units.map { (id, us) -> LinkUnitLatency(id, us) }
    )
}

data class SystemUiState(
    /** Null while not on a hub (or before the first answer). */
    val view: LinkSystemView? = null,
    /** Why the view could not be read; "" when it was. */
    val error: String = "",
    val busy: Boolean = false,
    val lastRecall: RecallLine? = null,
    /** The last scene actually taken, by name. */
    val recalledScene: String = "",
    val actionMessage: String = "",
    val actionFailed: Boolean = false,
    val draft: SystemDraft = SystemDraft(),
    /** The hub's revision moved past the draft's: Save is withdrawn, Reload offered. */
    val stale: Boolean = false
) {
    val available: Boolean get() = view != null
    val mayEdit: Boolean get() = view?.mayEdit == true
    val mayRecall: Boolean get() = view?.mayRecall == true

    /** The draft's value for a room's lip sync, as text ("" for the model's own). */
    fun roomLipSyncText(id: String): String = if (draft.rooms.containsKey(id)) {
        draft.rooms[id]?.toString().orEmpty()
    } else {
        view?.rooms?.firstOrNull { it.id == id }?.lipSyncUs?.toString().orEmpty()
    }

    /** The draft's value for a unit's measured latency, as text ("" for none). */
    fun unitLatencyText(id: String): String = if (draft.units.containsKey(id)) {
        draft.units[id]?.toString().orEmpty()
    } else {
        view?.units?.firstOrNull { it.id == id }?.measuredLatencyUs?.toString().orEmpty()
    }
}

/**
 * The System screen's model over [bridge]; actions run in [scope]. [poll]
 * reads the view every [POLL_MS] while the caller keeps it running (the
 * screen and Home's chip, while shown). The hub answers a session's status at
 * most once a second; the core then gives its last view again.
 */
class SystemScreenModel(private val bridge: DspiBridge, private val scope: CoroutineScope) {
    private val _state = MutableStateFlow(SystemUiState())
    val state: StateFlow<SystemUiState> = _state.asStateFlow()
    private val reading = Mutex()

    suspend fun poll(intervalMs: Long = POLL_MS) {
        while (true) {
            refresh()
            delay(intervalMs)
        }
    }

    suspend fun refresh() {
        if (reading.isLocked) return
        reading.withLock {
            val v = try {
                bridge.linkSystemView()
            } catch (e: CancellationException) {
                throw e
            } catch (e: DspiException.Disconnected) {
                _state.update { it.copy(view = null, error = "") }
                return
            } catch (e: DspiException) {
                _state.update { it.copy(error = words(e)) }
                return
            }
            _state.update { s ->
                val stale = s.stale || (s.draft.dirty && v.revision != s.draft.revision)
                val draft = if (s.draft.dirty) s.draft else SystemDraft(v.revision, false, v.systemDelayAuto, v.systemDelayFrames)
                s.copy(view = v, error = "", stale = stale, draft = draft)
            }
        }
    }

    private fun act(block: suspend () -> Unit) {
        if (_state.value.busy) return
        _state.update { it.copy(busy = true) }
        scope.launch {
            try {
                block()
            } finally {
                _state.update { it.copy(busy = false) }
            }
            refresh()
        }
    }

    fun recall(id: String) {
        val s = _state.value
        if (!s.mayRecall) return
        val name = s.view?.scenes?.firstOrNull { it.id == id }?.name ?: id
        act {
            val line = try {
                line(bridge.linkSystemRecall(id))
            } catch (e: CancellationException) {
                throw e
            } catch (e: DspiException) {
                RecallLine(name, false, "$name was refused: ${words(e)}", emptyList())
            }
            _state.update { it.copy(lastRecall = line, recalledScene = if (line.taken) line.name else it.recalledScene) }
        }
    }

    fun align() {
        if (!_state.value.mayEdit) return
        act {
            try {
                val r = bridge.linkSystemAlign()
                _state.update { it.copy(actionMessage = r.message, actionFailed = !r.applied) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: DspiException) {
                _state.update { it.copy(actionMessage = words(e), actionFailed = true) }
            }
        }
    }

    private fun edit(f: (SystemDraft) -> SystemDraft): Boolean {
        val s = _state.value
        if (!s.mayEdit) return false
        val base = if (s.draft.dirty) s.draft else s.draft.copy(revision = s.view?.revision ?: 0)
        _state.update { it.copy(draft = f(base)) }
        return true
    }

    fun setSystemDelay(automatic: Boolean, frames: Long) =
        edit { it.copy(delaySet = true, delayAuto = automatic, delayFrames = frames.coerceIn(0, 65_535)) }

    /** False for text that is not a whole number of µs ("" clears it). */
    fun setRoomLipSync(id: String, text: String): Boolean {
        val us = parseUs(text) ?: return false
        return edit { it.copy(rooms = it.rooms + (id to us.value)) }
    }

    fun setUnitLatency(id: String, text: String): Boolean {
        val us = parseUs(text) ?: return false
        return edit { it.copy(units = it.units + (id to us.value)) }
    }

    fun save() {
        val s = _state.value
        if (!s.draft.dirty || s.stale || !s.mayEdit) return
        act {
            try {
                bridge.linkSystemSave(s.draft.revision, s.draft.edits())
                _state.update { it.copy(draft = SystemDraft(), stale = false, actionMessage = "Saved on the hub.", actionFailed = false) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: DspiException) {
                val conflict = e is DspiException.Link && e.code == "conflict"
                _state.update { it.copy(stale = it.stale || conflict, actionMessage = words(e), actionFailed = true) }
            }
        }
    }

    /** Throws the draft away and reads the view again. */
    fun reload() {
        _state.update { it.copy(draft = SystemDraft(), stale = false, actionMessage = "", actionFailed = false) }
        scope.launch { refresh() }
    }

    companion object {
        const val POLL_MS = 3_000L

        /** A whole number of µs as typed: null when it is not one; a null value for "". */
        fun parseUs(text: String): Holder? {
            val t = text.trim()
            if (t.isEmpty()) return Holder(null)
            if (!Regex("-?\\d{1,7}").matches(t)) return null
            return Holder(t.toLong())
        }

        fun words(e: DspiException): String = when (e) {
            is DspiException.Link -> e.msg
            else -> e.message.orEmpty()
        }

        fun line(r: LinkSceneRecall) = RecallLine(r.name, r.taken, r.message, r.writes.map { "${it.unit} · ${it.what}: ${it.resultWord}" })
    }

    /** A parsed value that may itself be null ("the model's own"). */
    data class Holder(val value: Long?)
}
