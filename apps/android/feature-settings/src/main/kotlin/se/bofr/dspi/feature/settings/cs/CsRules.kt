// The control-surface pages' view of CG20's editing rules (DspiCoreApi.csDescribe,
// csEdit, csNewBinding): a record's rows as the core describes them, its replies
// parsed, and the operations the pages send. The pages only draw these: what a
// record offers is decided in the core (dspi_session::cs_rows), never here (WP AN9).
package se.bofr.dspi.feature.settings.cs

import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToLong
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.bool
import se.bofr.dspi.ui.json.double
import se.bofr.dspi.ui.json.int
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.obj
import se.bofr.dspi.ui.json.str

/** One choice of a [CsRow.Pick]: [owner] names who else holds a pin; [disabled] ones are shown, not offered. */
data class CsOption(val value: Int, val text: String, val section: String? = null, val owner: String? = null, val disabled: Boolean = false)

/** One row of a record's page, keyed by the core's stable [id]. */
sealed interface CsRow {
    val id: String
    val title: String
    val detail: String

    data class Pick(
        override val id: String,
        override val title: String,
        override val detail: String,
        val value: Int,
        val text: String,
        val options: List<CsOption>
    ) : CsRow

    /** A number in natural units (dB, Hz, seconds, percent). */
    data class Number(
        override val id: String,
        override val title: String,
        override val detail: String,
        val value: Double,
        val min: Double,
        val max: Double,
        val step: Double,
        val unit: String,
        val decimals: Int
    ) : CsRow

    data class Toggle(
        override val id: String,
        override val title: String,
        override val detail: String,
        val on: Boolean,
        val enabled: Boolean
    ) : CsRow

    /** Words only, in [detail]. */
    data class Note(override val id: String, override val title: String, override val detail: String, val warning: Boolean) : CsRow
}

/** What a page shows for one record. */
data class CsDescription(val title: String, val summary: String, val rows: List<CsRow>)

/** A rules call's answer: [value] on success, else [error] (the core's words). */
data class CsRuleReply<T>(val value: T?, val error: String?) {
    val ok: Boolean get() = value != null
}

/** How a pick row opens: a short menu, or a pushed page with sections, owners and (when long) a search field. */
enum class CsPickStyle { MENU, PAGE }

object CsRules {
    /** A menu longer than this is unusable on a phone (AN8 section 6): it becomes a page. */
    const val MENU_MAX = 6

    /** A picker page this long gets a search field (the function list: about 70 on firmware). */
    const val SEARCH_MIN = 12

    const val KIND_BINDING = "binding"

    private const val UNREADABLE = "the core's answer could not be read"

    /** `{ok, title, summary, rows}` from `csDescribe`. */
    fun parseDescription(json: String): CsRuleReply<CsDescription> {
        val m = Json.parseObject(json) ?: return CsRuleReply(null, UNREADABLE)
        if (m.bool("ok") != true) return CsRuleReply(null, m.str("error") ?: UNREADABLE)
        val rows = m.list("rows").orEmpty().mapNotNull { r ->
            @Suppress("UNCHECKED_CAST")
            (r as? Map<String, Any?>)?.let { row(it) }
        }
        return CsRuleReply(CsDescription(m.str("title") ?: "", m.str("summary") ?: "", rows), null)
    }

    /** `{ok, draft}` from `csEdit`, `csNewBinding` or `csNewRecord`: the draft as JSON text. */
    fun parseDraft(json: String): CsRuleReply<String> {
        val m = Json.parseObject(json) ?: return CsRuleReply(null, UNREADABLE)
        if (m.bool("ok") != true) return CsRuleReply(null, m.str("error") ?: UNREADABLE)
        val d = m.obj("draft") ?: return CsRuleReply(null, UNREADABLE)
        return CsRuleReply(Json.write(d), null)
    }

    private fun row(m: Map<String, Any?>): CsRow? {
        val id = m.str("id") ?: return null
        val title = m.str("title") ?: ""
        val detail = m.str("detail") ?: ""
        return when (m.str("kind")) {
            "pick" -> CsRow.Pick(
                id,
                title,
                detail,
                m.int("value") ?: 0,
                m.str("text") ?: "",
                m.list("options").orEmpty().mapNotNull { o ->
                    @Suppress("UNCHECKED_CAST")
                    (o as? Map<String, Any?>)?.let {
                        CsOption(
                            it.int("value") ?: 0,
                            it.str("text") ?: "",
                            it.str("section"),
                            it.str("owner"),
                            it.bool("disabled") ?: false
                        )
                    }
                }
            )
            "number" -> CsRow.Number(
                id,
                title,
                detail,
                m.double("value") ?: 0.0,
                m.double("min") ?: 0.0,
                m.double("max") ?: 0.0,
                m.double("step") ?: 1.0,
                m.str("unit") ?: "",
                (m.int("decimals") ?: 0).coerceIn(0, 6)
            )
            "toggle" -> CsRow.Toggle(id, title, detail, m.bool("on") ?: false, m.bool("enabled") ?: true)
            "note" -> CsRow.Note(id, title, detail, m.bool("warning") ?: false)
            // A kind this client does not know yet: shown as words rather than dropped
            else -> CsRow.Note(id, title, detail, false)
        }
    }

    /** The `change` argument of `csEdit`: an option's value, a number in natural units, or a boolean. */
    fun change(id: String, value: Any): String = Json.write(linkedMapOf("id" to id, "value" to value))

    fun pickStyle(row: CsRow.Pick): CsPickStyle =
        if (row.options.size > MENU_MAX || row.options.any { it.section != null || it.owner != null || it.disabled }) {
            CsPickStyle.PAGE
        } else {
            CsPickStyle.MENU
        }

    /** The options a search shows: [query] in the text or the section, case-insensitive; blank shows all. */
    fun filter(options: List<CsOption>, query: String): List<CsOption> {
        val q = query.trim()
        if (q.isEmpty()) return options
        return options.filter { it.text.contains(q, ignoreCase = true) || it.section?.contains(q, ignoreCase = true) == true }
    }

    /** Runs of options under one section header, in the core's order. */
    fun sections(options: List<CsOption>): List<Pair<String?, List<CsOption>>> {
        val out = mutableListOf<Pair<String?, MutableList<CsOption>>>()
        for (o in options) {
            if (out.lastOrNull()?.first == o.section) out.last().second += o else out += o.section to mutableListOf(o)
        }
        return out
    }

    /** A number row's value as the row shows it. */
    fun formatNumber(row: CsRow.Number): String {
        val v = String.format(Locale.ROOT, "%.${row.decimals}f", row.value)
        return if (row.unit.isEmpty()) v else "$v ${row.unit}"
    }

    /**
     * Typed number entry: the value, or why not. Accepts a comma as the decimal
     * mark and a trailing unit; refuses what is outside the row's range.
     */
    fun parseNumber(row: CsRow.Number, text: String): CsRuleReply<Double> {
        val t = text.trim().removeSuffix(row.unit).trim().replace(',', '.')
        val v = t.toDoubleOrNull()?.takeIf { it.isFinite() } ?: return CsRuleReply(null, "Not a number")
        val lo = minOf(row.min, row.max)
        val hi = maxOf(row.min, row.max)
        // The range is shown rounded; a value equal to an end at the shown decimals is inside it
        val slack = if (row.decimals > 0) 0.5 / Math.pow(10.0, row.decimals.toDouble()) else 0.5
        if (v < lo - slack || v > hi + slack) return CsRuleReply(null, "Out of range: ${rangeText(row)}")
        return CsRuleReply(v.coerceIn(lo, hi), null)
    }

    fun rangeText(row: CsRow.Number): String {
        fun f(d: Double) = if (row.decimals == 0 &&
            abs(d) < 1e15
        ) {
            d.roundToLong().toString()
        } else {
            String.format(Locale.ROOT, "%.${row.decimals}f", d)
        }
        return "${f(row.min)} to ${f(row.max)}" + if (row.unit.isEmpty()) "" else " ${row.unit}"
    }

    /** `cs.apply` for a binding: the draft (a `cs_snapshot` record) into [slot]; null for a draft that is not an object. */
    fun bindingOp(slot: Int, draft: String): String? {
        val d = Json.parseObject(draft) ?: return null
        return Json.write(linkedMapOf("op" to "binding", "index" to slot, "binding" to d))
    }

    /** Why an apply did not take, or null: the core's `error`, else the status code in words. */
    fun applyError(reply: String, statusText: (Int) -> String): String? {
        val m = Json.parseObject(reply) ?: return UNREADABLE
        if (m.bool("ok") == true) return null
        m.str("error")?.takeIf { it.isNotEmpty() }?.let { return it }
        val code = m.int("status") ?: 0xFF
        return statusText(code).ifEmpty { "Refused (status $code)" }
    }
}
