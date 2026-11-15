// WP P2AN: the History page's logic (PLAN-P2 M1), JVM-tested. Two sources:
// this console's own event log of the open device (`event_log_read`) and,
// on a hub with the `events` cap, the hub's shared history ("On the hub",
// `event.list`, with who did it in `by`), kept live while the page is open
// (`event.watch` then `event.added`). The records and their words are the
// core's; this holds which source is shown and the rows, newest first.
package se.bofr.dspi.feature.settings

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.EventEntry
import se.bofr.dspi.core.HubEventRecord

/** Where the History page reads from. */
enum class EventsSource { CONSOLE, HUB }

/** One line of the page: when, what, and who or what caused it ("" none). */
data class EventRow(val id: Long, val timeMs: Long, val kind: String, val text: String, val by: String) {
    companion object {
        fun of(e: EventEntry) = EventRow(e.id, e.timeMs, e.kind, e.text, if (e.own) THIS_CONSOLE else e.source)

        fun of(r: HubEventRecord) = EventRow(r.id, r.timeMs, r.kind, r.text, r.by.ifEmpty { r.source })

        /** Who caused this console's own writes. */
        const val THIS_CONSOLE = "this console"
    }
}

class EventsModel(private val bridge: DspiBridge, private val scope: CoroutineScope) {
    private val hub get() = bridge.hubServer

    private val _source = MutableStateFlow(EventsSource.CONSOLE)
    val source: StateFlow<EventsSource> = _source.asStateFlow()

    private val _rows = MutableStateFlow<List<EventRow>>(emptyList())

    /** Newest first. */
    val rows: StateFlow<List<EventRow>> = _rows.asStateFlow()

    private val _line = MutableStateFlow("")

    /** A failure to read, in the user's words ("" none). */
    val line: StateFlow<String> = _line.asStateFlow()

    private var live: Job? = null
    private var open = false

    /** The hub keeps a history this page can show ("On the hub" offered). */
    val hubOffered: Boolean get() = hub.caps.value.events

    /** The page is on screen: read the chosen source; the hub's stays live. */
    fun open() {
        open = true
        if (live == null) {
            live = scope.launch {
                hub.eventAdded.collect { r ->
                    if (_source.value == EventsSource.HUB && _rows.value.none { it.id == r.id }) {
                        _rows.value = (listOf(EventRow.of(r)) + _rows.value).take(MAX_ROWS)
                    }
                }
            }
        }
        refresh()
    }

    /** The page left the screen: the hub stops sending. */
    fun close() {
        open = false
        live?.cancel()
        live = null
        scope.launch { hub.watchEvents(false) }
    }

    fun choose(s: EventsSource) {
        if (s == EventsSource.HUB && !hubOffered) return
        if (_source.value == s) return
        _source.value = s
        _rows.value = emptyList()
        refresh()
    }

    fun refresh() {
        // A hub that lost the family (another session): back to this console's log
        if (_source.value == EventsSource.HUB && !hubOffered) _source.value = EventsSource.CONSOLE
        val s = _source.value
        scope.launch {
            _line.value = ""
            when (s) {
                EventsSource.CONSOLE -> {
                    val rows = hub.localEvents().map(EventRow::of)
                    if (_source.value == s) _rows.value = rows
                    hub.watchEvents(false)
                }
                EventsSource.HUB -> {
                    hub.hubEvents().onSuccess { p -> if (_source.value == s) _rows.value = p.records.asReversed().map(EventRow::of) }
                        .onFailure { _line.value = it.message ?: "The hub's history could not be read." }
                    if (open) hub.watchEvents(true)
                }
            }
        }
    }

    companion object {
        /** The page keeps at most this many rows. */
        const val MAX_ROWS = 500
    }
}
