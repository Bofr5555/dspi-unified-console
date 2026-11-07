// WP AN19: the onboarding words, read from the core's JSON (CG10,
// `dspi_core::link_text`). The core has already resolved them for this
// platform; what is left for a client is `{name}` values only it knows.
package se.bofr.dspi.feature.onboarding

import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.obj
import se.bofr.dspi.ui.json.str

/** Where the onboarding data comes from: the core's `onboarding_*_json`, read on demand (built-in text, no device). */
class OnboardingSource(val whatsNewJson: () -> String, val gettingStartedJson: () -> String) {
    companion object {
        /** No data: What's New is empty and Getting Started has no stages. */
        val EMPTY = OnboardingSource({ "[]" }, { "{}" })
    }
}

/** One release's notes (macOS `ReleaseNotes`). */
data class ReleaseNotes(val version: String, val headline: String, val items: List<String>)

/** A paragraph or a row of a stage; [icon] is a name the client maps to its own icons. */
data class OnboardingLine(val id: String, val text: String, val icon: String? = null)

/** The board stage's heading for one situation. */
data class OnboardingHeader(val id: String, val title: String, val blurb: String)

/** The board stage's card for one situation; [action] is its button, if it has one. */
data class OnboardingCard(val id: String, val title: String, val text: String, val action: String? = null)

/** One step of the wizard. A stage uses the lists it needs; the others are empty. */
data class OnboardingStage(
    val id: String,
    val label: String,
    val title: String = "",
    val paragraphs: List<OnboardingLine> = emptyList(),
    val rows: List<OnboardingLine> = emptyList(),
    val headers: List<OnboardingHeader> = emptyList(),
    val cards: List<OnboardingCard> = emptyList()
) {
    fun header(id: String): OnboardingHeader? = headers.firstOrNull { it.id == id }

    fun card(id: String): OnboardingCard? = cards.firstOrNull { it.id == id }
}

/** The Getting Started wizard's words. [buttons] by key: skip, back, continue, finish, try_again. */
data class GettingStartedWords(
    val title: String = "",
    val stepCounter: String = "",
    val buttons: Map<String, String> = emptyMap(),
    val stages: List<OnboardingStage> = emptyList()
) {
    fun button(key: String): String = buttons[key].orEmpty()

    fun stage(id: String): OnboardingStage? = stages.firstOrNull { it.id == id }
}

object OnboardingText {
    /** What's New, in the file's order; anything malformed is left out, never thrown. */
    fun parseWhatsNew(json: String): List<ReleaseNotes> {
        val releases = runCatching { Json.parse(json) as? List<*> }.getOrNull() ?: return emptyList()
        return releases.mapNotNull { r ->
            val m = r.asMap() ?: return@mapNotNull null
            val version = m.str("version") ?: return@mapNotNull null
            ReleaseNotes(version, m.str("headline").orEmpty(), m.list("items").orEmpty().mapNotNull { textOf(it) })
        }
    }

    /** Getting Started; empty words (no stages) when the text is not a document. */
    fun parseGettingStarted(json: String): GettingStartedWords {
        val root = Json.parseObject(json) ?: return GettingStartedWords()
        val buttons = root.obj("buttons").orEmpty().mapNotNull { (k, v) -> (v as? String)?.let { k to it } }.toMap()
        val stages = root.list("stages").orEmpty().mapNotNull { s ->
            val m = s.asMap() ?: return@mapNotNull null
            OnboardingStage(
                id = m.str("id") ?: return@mapNotNull null,
                label = m.str("label").orEmpty(),
                title = m.str("title").orEmpty(),
                paragraphs = lines(m.list("paragraphs")),
                rows = lines(m.list("rows")),
                headers = m.list("headers").orEmpty().mapNotNull { h ->
                    val hm = h.asMap() ?: return@mapNotNull null
                    OnboardingHeader(hm.str("id") ?: return@mapNotNull null, hm.str("title").orEmpty(), hm.str("blurb").orEmpty())
                },
                cards = m.list("cards").orEmpty().mapNotNull { c ->
                    val cm = c.asMap() ?: return@mapNotNull null
                    OnboardingCard(
                        cm.str("id") ?: return@mapNotNull null,
                        cm.str("title").orEmpty(),
                        cm.str("text").orEmpty(),
                        cm.str("action")
                    )
                }
            )
        }
        return GettingStartedWords(root.str("title").orEmpty(), root.str("step_counter").orEmpty(), buttons, stages)
    }

    /** [text] with each `{name}` of [values] filled in; a name not given stays as it is. */
    fun fill(text: String, values: Map<String, String>): String =
        values.entries.fold(text) { t, (name, value) -> t.replace("{$name}", value) }

    // The core turns an object left with `text` alone into a plain string
    private fun lines(items: List<Any?>?): List<OnboardingLine> = items.orEmpty().mapIndexedNotNull { i, item ->
        when (item) {
            is String -> OnboardingLine("#$i", item)
            else -> item.asMap()?.let { m -> m.str("text")?.let { OnboardingLine(m.str("id") ?: "#$i", it, m.str("icon")) } }
        }
    }

    private fun textOf(item: Any?): String? = item as? String ?: item.asMap()?.str("text")

    @Suppress("UNCHECKED_CAST")
    private fun Any?.asMap(): Map<String, Any?>? = this as? Map<String, Any?>
}
