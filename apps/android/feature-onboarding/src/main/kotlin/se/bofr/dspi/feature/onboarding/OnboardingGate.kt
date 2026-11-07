// WP AN19: what to show at launch, as a pure function of the stored state, the
// release notes and this build's version (macOS OnboardingCoordinator and
// `WhatsNew.unread`; Qt main.qml `startOnboarding`). Nothing here draws.
package se.bofr.dspi.feature.onboarding

import se.bofr.dspi.ui.settings.AppSettings

/**
 * How two release versions compare: negative, zero or positive, or null when
 * either is not a version. The app supplies the firmware version rules, so a
 * beta sorts below its release.
 */
fun interface ReleaseOrder {
    fun compare(a: String, b: String): Int?
}

object OnboardingGate {
    enum class Show { NOTHING, GETTING_STARTED, WHATS_NEW }

    /** What to show, and what to store for it ([update] is applied whatever [show] is). */
    class Launch(val show: Show, val update: (AppSettings) -> AppSettings)

    /**
     * At launch. [current] is the release this build is ([ReleaseOrder] form), null if unknown.
     * - Setup not done: Getting Started. A new install starts with the notes read, as on
     *   macOS, since a list of changes from a version never run is noise.
     * - Notes never shown (the app ran before What's New existed): marked read, nothing shown.
     * - Otherwise What's New when a release after the one last shown, up to [current], has notes.
     * Starting from Settings > About > Help is not this: the screens are opened directly.
     */
    fun atLaunch(settings: AppSettings, notes: List<ReleaseNotes>, current: String?, order: ReleaseOrder): Launch {
        val markRead: (AppSettings) -> AppSettings =
            { s -> if (current == null) s else s.copy(whatsNewLastShownVersion = current) }
        return when {
            !settings.onboardingSetupDone -> {
                val firstNotes = settings.whatsNewLastShownVersion.isEmpty()
                Launch(Show.GETTING_STARTED) { s -> if (firstNotes) markRead(s) else s }
            }
            settings.whatsNewLastShownVersion.isEmpty() -> Launch(Show.NOTHING, markRead)
            unread(notes, settings.whatsNewLastShownVersion, current, order).isNotEmpty() -> Launch(Show.WHATS_NEW, markRead)
            else -> Launch(Show.NOTHING) { it }
        }
    }

    /**
     * Notes after [shown] and no later than [current] (macOS `WhatsNew.unread`): the notes are
     * written while a release is being built, so they can describe one this build is not yet.
     */
    fun unread(notes: List<ReleaseNotes>, shown: String, current: String?, order: ReleaseOrder): List<ReleaseNotes> {
        if (current == null || shown.isEmpty()) return emptyList()
        return notes.filter { r ->
            val afterShown = order.compare(r.version, shown) ?: return@filter false
            val upToCurrent = order.compare(r.version, current) ?: return@filter false
            afterShown > 0 && upToCurrent <= 0
        }
    }

    /** Newest first whatever the file's order; a version that does not parse goes last (macOS `WhatsNew.load`). */
    fun newestFirst(notes: List<ReleaseNotes>, order: ReleaseOrder): List<ReleaseNotes> {
        val (known, unknown) = notes.partition { order.compare(it.version, it.version) != null }
        return known.sortedWith { a, b -> order.compare(b.version, a.version) ?: 0 } + unknown
    }
}
