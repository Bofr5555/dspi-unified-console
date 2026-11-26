// -- WP UAN -- (UXP B8, owner D9) Settings › Speakers on a phone: which output drives what, one row
// per output ("3 · None ▸ Role · Side"), from the kept layout or the filters' suggestion, kept by
// the user's Confirm. The core keeps the layout and decides what a role means (the tweeter rule,
// the preset load's guard, the measurement); this page only lets the user say it.
package se.bofr.dspi.feature.settings

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.OutputLimits
import se.bofr.dspi.core.MeasureRole
import se.bofr.dspi.core.RoleSource
import se.bofr.dspi.core.SpeakerLayout
import se.bofr.dspi.core.SpeakerLayoutOutput

/** One output's row: [role] null is None (the output is left out of the layout). */
data class SpeakerRow(val output: Int, val label: String, val role: MeasureRole?, val side: String)

/** The page's state: the rows, where they come from, and a refusal in the core's words. */
data class SpeakersState(
    val rows: List<SpeakerRow> = emptyList(),
    /** The kept layout is the user's (confirmed); a draft the user changed is not, until Confirm. */
    val confirmed: Boolean = false,
    /** The speaker detection's layout, not confirmed: its roles guard the outputs already. */
    val detected: Boolean = false,
    /** The rows differ from what is kept. */
    val changed: Boolean = false,
    val busy: Boolean = false,
    val error: String = "",
    /** A device is open. */
    val connected: Boolean = false
)

object SpeakerWords {
    /** The role menu, in the order the page offers it; null is None. */
    val roles: List<MeasureRole?> = listOf(MeasureRole.HIGH, MeasureRole.MID, MeasureRole.LOW, MeasureRole.SUB, MeasureRole.FULL, null)

    fun role(r: MeasureRole?): String = when (r) {
        MeasureRole.HIGH -> "Tweeter"
        MeasureRole.MID -> "Mid"
        MeasureRole.LOW -> "Woofer"
        MeasureRole.SUB -> "Sub"
        MeasureRole.FULL -> "Full range"
        null -> "None"
    }

    /** The sides a row offers; a kept speaker of the user's own words is offered too. */
    val sides = listOf("L", "R", "C")

    const val TITLE = "Which output drives what?"
    const val INTRO = "Tell uConsole what each output plays: a full-range speaker, a woofer, a mid, a tweeter or a subwoofer. " +
        "The suggestion comes from the DSPi's filters and routing: check it and confirm it."
    const val SAFETY = "A tweeter or a mid gets the protection of its role: a preset that would leave it without its high-pass " +
        "is held, and it is measured only with one. An output with an active high-pass at 1 kHz or above is treated as a " +
        "tweeter, whatever its role here."
    const val MEASURE = "Measuring and auto-tune are in the desktop and web consoles. The roles you confirm here are kept in this " +
        "app for this DSPi."
    const val CONFIRMED = "Confirmed. You can change it here at any time."
    const val DETECTED = "Found by the speaker detection, not confirmed yet: its roles protect the outputs already. Check them and confirm."
    const val UNCONFIRMED = "Not confirmed: a suggestion from the filters and routing. Check it and confirm it; until then no role " +
        "protects an output."
    const val NO_DEVICE = "Connect your DSPi to set this up."
    const val LATER = "Not sure yet? Choose Later: it waits for you in Settings › Speakers."
}

/**
 * The rule's corner and slope a role puts on its output's high-pass, in words, null for none: a tweeter's
 * at least 1 kHz and 12 dB/octave (a limits profile may raise them, never lower them), a mid's 80 Hz
 * and 12 dB/octave unless the profile names its own. The core's `tweeter_rule` (measure/preflight.rs)
 * is the judge; this only says it, and its test holds the two the same.
 */
fun speakerFloorWords(role: MeasureRole?, limits: OutputLimits?): String? {
    val (hz, slope) = when (role) {
        MeasureRole.HIGH -> maxOf(limits?.highpassMinHz ?: TWEETER_MIN_HZ, TWEETER_MIN_HZ) to
            maxOf(limits?.highpassMinSlopeDb ?: MIN_SLOPE_DB, MIN_SLOPE_DB)
        MeasureRole.MID -> (limits?.highpassMinHz ?: MID_MIN_HZ) to (limits?.highpassMinSlopeDb ?: MIN_SLOPE_DB)
        else -> return null
    }
    return "Its high-pass: at least ${hz.toInt()} Hz, $slope dB/octave"
}

/** The core's floors (measure/preflight.rs HIGH_MIN_HZ, MID_MIN_HZ, MIN_SLOPE_DB). */
const val TWEETER_MIN_HZ = 1000f
const val MID_MIN_HZ = 80f
const val MIN_SLOPE_DB = 12

/** [layout]'s rows over every output of the device, [labels] by output; an output it leaves out is None. */
fun speakerRows(layout: SpeakerLayout?, outputs: Int, labels: (Int) -> String): List<SpeakerRow> = (0 until outputs).map { o ->
    val row = layout?.outputs?.firstOrNull { it.output == o }
    SpeakerRow(o, labels(o), row?.role, row?.speaker?.ifBlank { null } ?: defaultSide(o))
}

/** A row with no speaker yet: odd outputs (1, 3, …) left, even ones right, as most boards pair them. */
fun defaultSide(output: Int) = if (output % 2 == 0) "L" else "R"

/** The layout the rows say: the outputs with a role, in order; one group per speaker, in the order met. */
fun layoutOf(rows: List<SpeakerRow>, serial: String): SpeakerLayout {
    val kept = rows.filter { it.role != null }
    val groups = kept.map { it.side.trim().lowercase() }.distinct()
    return SpeakerLayout(
        serial = serial,
        outputs = kept.map { r ->
            SpeakerLayoutOutput(r.output, r.role!!, r.side.trim(), groups.indexOf(r.side.trim().lowercase()), RoleSource.USER)
        },
        confirmedByUser = true,
        updatedMs = 0,
        notes = emptyList(),
        detected = null,
        stated = null
    )
}

/**
 * "Same for the right side": each pair's right output (by [pairs], left to right) takes its left
 * output's role, on the side R when the left one is L. Rows of other outputs stay.
 */
fun sameForRight(rows: List<SpeakerRow>, pairs: List<Pair<Int, Int>>): List<SpeakerRow> {
    val byOutput = rows.associateBy { it.output }.toMutableMap()
    for ((l, r) in pairs) {
        val left = byOutput[l] ?: continue
        val right = byOutput[r] ?: continue
        byOutput[r] = right.copy(role = left.role, side = if (left.side == "L") "R" else right.side)
    }
    return rows.map { byOutput.getValue(it.output) }
}

/** The page's model over [bridge]'s speaker layout (main thread). */
class SpeakersModel(private val bridge: DspiBridge, private val scope: CoroutineScope) {
    private val _state = MutableStateFlow(SpeakersState())
    val state: StateFlow<SpeakersState> = _state.asStateFlow()
    private var kept: List<SpeakerRow> = emptyList()

    private fun labelOf(o: Int) = bridge.channelLabel(2 + o).ifBlank { "Out ${o + 1}" }

    /** Read the kept layout, else the suggestion. */
    fun load() {
        val connected = bridge.connected
        if (!connected) {
            _state.value = SpeakersState(connected = false)
            return
        }
        _state.value = _state.value.copy(busy = true, connected = true)
        scope.launch {
            val current = bridge.speakers.current()
            val layout = current.getOrNull() ?: bridge.speakers.propose().getOrNull()
            val rows = speakerRows(layout, bridge.numOutputChannels, ::labelOf)
            kept = if (current.getOrNull() != null) rows else emptyList()
            _state.value = SpeakersState(
                rows = rows,
                confirmed = current.getOrNull()?.confirmedByUser == true,
                detected = current.getOrNull()?.let { !it.confirmedByUser && it.detected != null } == true,
                changed = false,
                connected = true,
                error = current.exceptionOrNull()?.message.orEmpty()
            )
        }
    }

    private fun edit(rows: List<SpeakerRow>) {
        _state.value = _state.value.copy(rows = rows, changed = rows != kept, error = "")
    }

    fun setRole(output: Int, role: MeasureRole?) = edit(_state.value.rows.map { if (it.output == output) it.copy(role = role) else it })

    fun setSide(output: Int, side: String) = edit(_state.value.rows.map { if (it.output == output) it.copy(side = side) else it })

    fun sameForRight(pairs: List<Pair<Int, Int>>) = edit(sameForRight(_state.value.rows, pairs))

    /** The filters' suggestion in place of the rows (not kept until Confirm). */
    fun useSuggestion() {
        scope.launch {
            bridge.speakers.propose().onSuccess { edit(speakerRows(it, bridge.numOutputChannels, ::labelOf)) }
                .onFailure { e -> _state.value = _state.value.copy(error = e.message.orEmpty()) }
        }
    }

    /** Keep the rows as the user's word; [then] after it is kept. */
    fun confirm(then: () -> Unit = {}) {
        val s = _state.value
        _state.value = s.copy(busy = true, error = "")
        scope.launch {
            bridge.speakers.confirm(layoutOf(s.rows, bridge.selectedSerial))
                .onSuccess { l ->
                    kept = speakerRows(l, bridge.numOutputChannels, ::labelOf)
                    _state.value = _state.value.copy(rows = kept, confirmed = true, detected = false, changed = false, busy = false)
                    then()
                }
                .onFailure { e -> _state.value = _state.value.copy(busy = false, error = e.message.orEmpty()) }
        }
    }
}
