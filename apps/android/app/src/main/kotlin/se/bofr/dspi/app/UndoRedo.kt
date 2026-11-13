// -- WP AN42 -- Undo and Redo in the strip at the top of the app, over the
// core's journal (P0W, docs/integrator/undo.md). As Qt's HistoryController
// (Edit > Undo / Redo, its notices) and the web's buttons beside the tabs
// (WB8u): the core decides what may be undone, steps over the rest and leaves
// another writer's change alone; the app shows what happened in a snackbar.
package se.bofr.dspi.app

import androidx.compose.foundation.layout.Row
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HistoryStep
import se.bofr.dspi.core.HistoryEntry
import se.bofr.dspi.ui.DspiIcons

/** Test tag of the strip's Undo button. */
internal const val UNDO_BUTTON_TAG = "undo"

/** Test tag of the strip's Redo button. */
internal const val REDO_BUTTON_TAG = "redo"

/** What the strip's two buttons can do: the journal's and G1's (read-only, a viewer, no device) answer together. */
data class UndoBarState(val canUndo: Boolean = false, val canRedo: Boolean = false)

/**
 * The view-model of Undo and Redo: the buttons' state, the steps, and the
 * words for each step's result ([messages], for the snackbar). The buttons and
 * Ctrl+Z / Ctrl+Shift+Z both come here.
 */
class UndoRedoModel(private val bridge: DspiBridge, private val scope: CoroutineScope) {
    val state: StateFlow<UndoBarState> =
        combine(bridge.undoState, bridge.writesAllowed) { u, w -> UndoBarState(w && u.canUndo, w && u.canRedo) }
            .stateIn(scope, SharingStarted.Eagerly, current())

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)

    /** One text per step, for the snackbar. */
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    private fun current(): UndoBarState {
        val w = bridge.writesAllowed.value
        val u = bridge.undoState.value
        return UndoBarState(w && u.canUndo, w && u.canRedo)
    }

    /** Step back; false (nothing sent) while there is nothing to undo or writes are not allowed. */
    fun undo(): Boolean = step(forward = false)

    /** Step forward again; false as [undo]. */
    fun redo(): Boolean = step(forward = true)

    private fun step(forward: Boolean): Boolean {
        val now = current()
        if (!(if (forward) now.canRedo else now.canUndo)) return false
        // Each press is its own step, in order: the bridge runs them one after another
        scope.launch {
            val r = if (forward) bridge.redo() else bridge.undo()
            historyMessage(forward, r)?.let { _messages.emit(it) }
        }
        return true
    }
}

/**
 * The snackbar's words for a step: what moved, then the core's note naming
 * another writer whose change was left alone, or else the changes stepped
 * over (Qt's and the web's wording). Null for a refused step: the bridge has
 * already said why ([HistoryStep.Refused]).
 */
fun historyMessage(forward: Boolean, step: HistoryStep): String? = when (step) {
    HistoryStep.Refused -> null
    is HistoryStep.Failed -> {
        val why = step.error.message?.takeIf { it.isNotBlank() } ?: step.error::class.simpleName.orEmpty()
        (if (forward) "Could not redo" else "Could not undo") + if (why.isEmpty()) "" else ": $why"
    }
    is HistoryStep.Moved -> {
        val m = step.move
        val lines = mutableListOf<String>()
        val entry = m.entry
        lines +=
            when {
                entry != null -> (if (forward) "Redone: " else "Undone: ") + historyTitle(entry)
                m.skipped.isEmpty() && m.note == null -> if (forward) "Nothing to redo" else "Nothing to undo"
                else -> if (forward) "Nothing was redone" else "Nothing was undone"
            }
        val note = m.note?.takeIf { it.isNotBlank() }
        if (note != null) {
            lines += note.replaceFirstChar { it.uppercaseChar() }
        } else if (m.skipped.isNotEmpty()) {
            val names = m.skipped.map(::historyTitle).distinct()
            lines +=
                if (names.size == 1) {
                    "Stepped over a change that cannot be undone: ${names[0]}"
                } else {
                    "Stepped over changes that cannot be undone: ${names.joinToString("; ")}"
                }
        }
        lines.joinToString("\n")
    }
}

/** The glossary's words for a journal path (docs/GLOSSARY.md; Qt's `HistoryController::labelOf`); an unknown path shows as itself. */
internal fun historyLabel(path: String): String =
    EXACT_LABELS[path] ?: FEATURE_LABELS.firstOrNull { path.startsWith(it.first) }?.second ?: path

/** A change's title: its label and where (`Gain, channel 1, band 3`), as Qt's `HistoryController::describe`. */
fun historyTitle(e: HistoryEntry): String {
    val p = e.path
    val ix = e.indices
    val where =
        when {
            p.startsWith("out.") && ix.isNotEmpty() -> listOf("OUT${ix[0] + 1}")
            (p.startsWith("eq.") || p.startsWith("ch.") || p == "pre" || p == "bypass") && ix.isNotEmpty() ->
                listOf("channel ${ix[0] + 1}") + if (p.startsWith("eq.") && ix.size > 1) listOf("band ${ix[1] + 1}") else emptyList()
            p == "mix" && ix.size > 1 -> listOf("IN${ix[0] + 1} to OUT${ix[1] + 1}")
            else -> emptyList()
        }
    return (listOf(historyLabel(p)) + where).joinToString(", ")
}

private val EXACT_LABELS =
    mapOf(
        "vol.user" to "Volume",
        "vol.master" to "Master volume",
        "vol.mute" to "Mute",
        "pre" to "Input trim",
        "bypass" to "EQ bypass",
        "eq.band" to "Filter band",
        "eq.type" to "Filter type",
        "eq.freq" to "Frequency",
        "eq.q" to "Q",
        "eq.gain" to "Gain",
        "eq.bypass" to "Band bypass",
        "ch.delay" to "Delay",
        "ch.name" to "Channel name",
        "mix" to "Crosspoint",
        "out.gain" to "Output gain",
        "out.mute" to "Output mute",
        "out.delay" to "Delay",
        "out.enable" to "Output on",
        "in.source" to "Input source",
        "dev.name" to "Device name"
    )

private val FEATURE_LABELS =
    listOf(
        "limit." to "Output Limiter",
        "loud." to "Loudness",
        "lev." to "Leveller",
        "cf." to "Crossfeed",
        "ext.pairmap" to "Crossfeed",
        "bass." to "Psychoacoustic Bass",
        "up." to "Upmixer",
        "sub." to "Subharmonic Synthesizer",
        "tube." to "Tube Modeller",
        "preset." to "Preset",
        "cs." to "Control surface"
    )

/** Undo and Redo, side by side in the strip; disabled while there is nothing to step or writes are not allowed. */
@Composable
internal fun UndoRedoButtons(model: UndoRedoModel, tint: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    val s by model.state.collectAsStateWithLifecycle()
    Row {
        IconButton(onClick = { model.undo() }, enabled = s.canUndo, modifier = Modifier.testTag(UNDO_BUTTON_TAG)) {
            Icon(DspiIcons.undo, contentDescription = "Undo", tint = if (s.canUndo) tint else tint.copy(alpha = 0.38f))
        }
        IconButton(onClick = { model.redo() }, enabled = s.canRedo, modifier = Modifier.testTag(REDO_BUTTON_TAG)) {
            Icon(DspiIcons.redo, contentDescription = "Redo", tint = if (s.canRedo) tint else tint.copy(alpha = 0.38f))
        }
    }
}

/** Shows each step's [UndoRedoModel.messages] in the app's snackbar; a newer one replaces the one showing. */
@Composable
internal fun UndoRedoNotices(model: UndoRedoModel, host: SnackbarHostState) {
    LaunchedEffect(model) { model.messages.collectLatest { host.showSnackbar(it, duration = SnackbarDuration.Short) } }
}
