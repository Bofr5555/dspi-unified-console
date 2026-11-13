// -- WP AN42 -- the drag mark: a slider or graph drag is one undo step
package se.bofr.dspi.ui

import androidx.compose.runtime.staticCompositionLocalOf

/**
 * Tells the core when continuous drags start and end (`DspiBridge.setDragging`),
 * so that every write between joins one undo step. Each control holds the mark
 * under its own [holder]; the core hears true when the first drag starts and
 * false when the last one ends (two fingers on two sliders are one mark, as the
 * web's counted mark). Main thread only.
 *
 * A drag's end must always land: a control ends its hold on release, on a
 * cancelled gesture and when it leaves the composition; the app calls
 * [endAll] when it stops (lifecycle) or the bridge goes. An end after
 * [endAll], or for a holder that holds nothing, does nothing.
 */
class DragMarker(private val send: (Boolean) -> Unit) {
    private val holders = LinkedHashSet<Any>()

    /** Whether any drag holds the mark. */
    val active: Boolean get() = holders.isNotEmpty()

    /** [holder]'s drag started (a second start of the same holder is ignored). */
    fun begin(holder: Any) {
        if (holders.add(holder) && holders.size == 1) send(true)
    }

    /** [holder]'s drag ended, was cancelled or left the screen. */
    fun end(holder: Any) {
        if (holders.remove(holder) && holders.isEmpty()) send(false)
    }

    /** Every drag ends now (the app stopped, the bridge changed). */
    fun endAll() {
        if (holders.isEmpty()) return
        holders.clear()
        send(false)
    }
}

/** The app's [DragMarker]; null where nothing records drags (previews, feature tests). */
val LocalDragMarker = staticCompositionLocalOf<DragMarker?> { null }
