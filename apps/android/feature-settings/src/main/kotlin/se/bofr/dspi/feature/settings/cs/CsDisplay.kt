// The display's part of the Control Surfaces snapshot (`cs_snapshot`'s `display`):
// its settings record, its pages and the panel's live state, with the operations
// the display pages send. What a page or the settings offer is the core's
// (CG20's `page` and `display` records); only the panel-state words are here,
// as the Mac's `displayStatusRow` and the Terminal word them (WP AN11).
package se.bofr.dspi.feature.settings.cs

import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.obj

/** The panel's live state (`REQ_GET_CS_DISPLAY_STATUS`). [page] is [CsDisplay.PAGE_NONE] when no page is up. */
data class CsDisplayStatus(
    val init: Int = 0,
    val page: Int = CsDisplay.PAGE_NONE,
    val flags: Int = 0,
    val model: Int = 0,
    val naks: Int = 0
)

/**
 * [config] and each of [pages] are JSON text in the shape `csDescribe` takes as a
 * draft; [pages] holds [maxPages] records, an inactive one empty.
 */
data class CsDisplayState(
    val maxPages: Int = 0,
    val config: String = "{}",
    val pages: List<String> = emptyList(),
    val status: CsDisplayStatus = CsDisplayStatus()
) {
    fun isActive(i: Int): Boolean = pages.getOrNull(i)?.let { CsDisplay.pageActive(it) } == true

    val activePages: List<Int> get() = pages.indices.filter { isActive(it) }

    /** Where Add puts a page, or null when every slot is in use. */
    val firstFreePage: Int? get() = pages.indices.firstOrNull { !isActive(it) }
}

object CsDisplay {
    const val KIND_PAGE = "page"
    const val KIND_DISPLAY = "display"

    /** `CsDisplayStatus.currentPage` when nothing (or a pop-up) is on the panel. */
    const val PAGE_NONE = 0xFF

    /** `CsDisplayPage.flags`: the slot is in use. */
    private const val PAGE_ACTIVE = 0x01

    // `CsDisplayStatus.initState`
    const val STATE_DOWN = 0
    const val STATE_INIT = 1
    const val STATE_LIVE = 2
    const val STATE_ERROR = 3

    /** The poll of the panel's state while its page is open: one 8-byte read a second, as the Mac. */
    const val STATUS_POLL_MS = 1_000L

    /** The `display` object of a `cs_snapshot`; "" or a snapshot without one gives no pages. */
    fun parse(snapshot: String): CsDisplayState {
        val d = Json.parseObject(snapshot)?.obj("display") ?: return CsDisplayState()
        val max = (d.int("maxPages") ?: 0).coerceAtLeast(0)
        val pages = d.list("pages").orEmpty().take(max).map { p ->
            @Suppress("UNCHECKED_CAST")
            (p as? Map<String, Any?>)?.let { Json.write(it) } ?: EMPTY_PAGE
        }
        val s = d.obj("status") ?: emptyMap()
        return CsDisplayState(
            maxPages = max,
            config = d.obj("config")?.let { Json.write(it) } ?: "{}",
            pages = pages + List((max - pages.size).coerceAtLeast(0)) { EMPTY_PAGE },
            status = CsDisplayStatus(
                s.int("init") ?: STATE_DOWN,
                s.int("page") ?: PAGE_NONE,
                s.int("flags") ?: 0,
                s.int("model") ?: 0,
                s.int("naks") ?: 0
            )
        )
    }

    fun pageActive(page: String): Boolean = ((Json.parseObject(page)?.int("flags") ?: 0) and PAGE_ACTIVE) != 0

    /** An all-zero page: written to a slot, it removes that page. */
    val EMPTY_PAGE: String = Json.write(linkedMapOf("noun" to 0, "target" to 0, "index" to 0, "flags" to 0))

    /** `cs.apply` for page [index]; null for a draft that is not an object. */
    fun pageOp(index: Int, page: String): String? {
        val p = Json.parseObject(page) ?: return null
        return Json.write(linkedMapOf("op" to "displayPage", "index" to index, "page" to p))
    }

    /** `cs.apply` that removes page [index]. */
    fun clearPageOp(index: Int): String =
        Json.write(linkedMapOf("op" to "displayPage", "index" to index, "page" to Json.parseObject(EMPTY_PAGE)))

    /** `cs.apply` for the display settings; null for a draft that is not an object. */
    fun configOp(config: String): String? {
        val c = Json.parseObject(config) ?: return null
        return Json.write(linkedMapOf("op" to "displayConfig", "config" to c))
    }

    /** The operation that applies a draft of [kind] ([KIND_PAGE] or [KIND_DISPLAY]). */
    fun op(kind: String, slot: Int, draft: String): String? = when (kind) {
        KIND_PAGE -> pageOp(slot, draft)
        KIND_DISPLAY -> configOp(draft)
        else -> null
    }

    val STATUS_READ: String = Json.write(linkedMapOf("op" to "displayStatus"))

    /** The panel state's words, and whether they warn. */
    fun stateText(s: CsDisplayStatus): Pair<String, Boolean> = when (s.init) {
        STATE_LIVE -> "Running" to false
        STATE_INIT -> "Starting up" to false
        STATE_ERROR -> "Not responding" to true
        else -> "Not started" to false
    }

    fun stateDetail(s: CsDisplayStatus): String = if (s.naks > 0) {
        "${s.naks} I2C error${if (s.naks == 1) "" else "s"} so far. Check wiring, pull-up resistors, and the address."
    } else {
        "Reported by the device."
    }
}
