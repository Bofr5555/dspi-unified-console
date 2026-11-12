// WP DISC2: the removal grace the Rust core's browser (dspi-link `HubList`) gives every
// desktop client, for this app's own NsdManager browser.
package se.bofr.dspi.link

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * A hub that is lost stays listed for [graceMs], and leaves only if it is not found again
 * meanwhile. A TXT change through Windows' DNS-SD is a goodbye and a new registration a
 * second or two later, and NsdManager reports a service lost and found now and then; a
 * list that showed each of those would flicker. Same value as the core's `REMOVAL_GRACE`.
 *
 * Not thread-safe: call it on [scope]'s thread, where [remove] runs too.
 */
internal class RemovalGrace(
    private val scope: CoroutineScope,
    private val graceMs: Long = REMOVAL_GRACE_MS,
    private val remove: (String) -> Unit
) {
    private val leaving = HashMap<String, Job>()

    /** [name] said goodbye: [remove] it after the grace, unless it is [back] first. */
    fun lost(name: String) {
        // A second goodbye does not put the end off.
        if (leaving.containsKey(name)) return
        leaving[name] = scope.launch {
            delay(graceMs)
            leaving.remove(name)
            remove(name)
        }
    }

    /** [name] was found again. */
    fun back(name: String) {
        leaving.remove(name)?.cancel()
    }

    /** Forget every pending removal (browsing stopped). */
    fun clear() {
        leaving.values.forEach { it.cancel() }
        leaving.clear()
    }

    val pending: Int get() = leaving.size

    companion object {
        const val REMOVAL_GRACE_MS = 3_000L
    }
}
