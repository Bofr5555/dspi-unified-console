// WP P1AN1: P1DF's diff (`config_diff`, `config_preview`, `ab_diff`) read for
// the screens, and the import blocks of PRO1 W8 (`IMPORT_*` bits). The rows,
// their words and the blocks are the core's; this only reads its JSON.
package se.bofr.dspi.feature.presets

import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.bool
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.str

/** One changed value: the import block that carries it, where it is, and the value before and after. */
data class DiffRow(val block: String, val section: String, val item: String, val before: String, val after: String)

/** P1DF's diff JSON (`{ok, error, rows, count, blocks, summary, notes, kind}`). */
data class DocDiff(
    val ok: Boolean,
    val error: String = "",
    val rows: List<DiffRow> = emptyList(),
    val summary: List<String> = emptyList(),
    val notes: List<String> = emptyList()
) {
    /** Rows per block key. */
    val countByBlock: Map<String, Int> get() = rows.groupingBy { it.block }.eachCount()

    companion object {
        fun parse(json: String): DocDiff {
            val m = Json.parseObject(json) ?: return DocDiff(false, "The changes could not be read.")
            if (m.bool("ok") != true) return DocDiff(false, m.str("error")?.ifEmpty { null } ?: "The changes could not be listed.")
            val rows =
                m.list("rows").orEmpty().filterIsInstance<Map<String, Any?>>().map { r ->
                    DiffRow(
                        block = r.str("block").orEmpty(),
                        section = r.str("section").orEmpty(),
                        item = r.str("item").orEmpty(),
                        before = text(r["before"]),
                        after = text(r["after"])
                    )
                }
            return DocDiff(
                ok = true,
                rows = rows,
                summary = m.list("summary").orEmpty().filterIsInstance<String>(),
                notes = m.list("notes").orEmpty().filterIsInstance<String>()
            )
        }

        private fun text(v: Any?): String = when (v) {
            null -> ""
            is Double -> if (v == Math.floor(v) && !v.isInfinite() && kotlin.math.abs(v) < 1e15) v.toLong().toString() else v.toString()
            else -> v.toString()
        }
    }
}

/** The blocks an import can apply alone (PRO1 W8), in the core's order. [key] is the diff rows' `block`. */
enum class ImportBlock(val key: String, val title: String, val bit: Int) {
    EQ("eq", "EQ", DspiConstants.IMPORT_EQ),
    CROSSOVER("crossover", "Crossover", DspiConstants.IMPORT_CROSSOVER),
    ROUTING("routing", "Routing and matrix", DspiConstants.IMPORT_ROUTING),
    OUTPUTS("outputs", "Outputs", DspiConstants.IMPORT_OUTPUTS),
    LIMITERS("limiters", "Limiters", DspiConstants.IMPORT_LIMITERS),
    EFFECTS("effects", "Effects", DspiConstants.IMPORT_EFFECTS),
    VOLUMES("volumes", "Volume levels", DspiConstants.IMPORT_VOLUMES),
    HARDWARE("hardware", "Hardware I/O", DspiConstants.IMPORT_HARDWARE),
    NAMES("names", "Names", DspiConstants.IMPORT_NAMES);

    companion object {
        /** Every block's bit, the volumes and the hardware I/O included: the preview lists every change. */
        val ALL_BITS: Long = entries.fold(0L) { acc, b -> acc or b.bit.toLong() }

        fun of(key: String): ImportBlock? = entries.firstOrNull { it.key == key }

        fun bits(blocks: Set<ImportBlock>): Long = blocks.fold(0L) { acc, b -> acc or b.bit.toLong() }

        /**
         * What is ticked at first: every block with a change, except the volume
         * levels and the hardware I/O, which an import left alone unless asked (as before).
         */
        fun defaults(diff: DocDiff): Set<ImportBlock> {
            val changed = diff.countByBlock.keys.mapNotNull(::of).toSet()
            val base = if (diff.ok) changed else entries.toSet()
            return base - VOLUMES - HARDWARE
        }
    }
}
