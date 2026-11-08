// WP AN26: the core's About table (CG15, `assets/about.json`), read for the About page.
// The words are the file's. Nothing here depends on what they say.
package se.bofr.dspi.feature.settings

import androidx.compose.runtime.staticCompositionLocalOf
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.str

/**
 * Reads the core's `aboutJson()`. The app provides it from the real core, not through the
 * bridge: demo mode's bridge sits on FakeDspiCore, and About reads the same there. Without a
 * provider (previews, tests) the table is empty and the page shows the rest.
 */
val LocalAboutJson = staticCompositionLocalOf<() -> String> { { "" } }

internal data class AboutMaker(val name: String, val url: String?, val made: List<String>)

internal data class AboutLink(val id: String, val title: String, val detail: String?, val url: String?)

internal data class AboutCredit(val name: String, val what: String?, val url: String?)

// -- WP LIC1 -- the project's licence and its GPL v3 section 7 terms (the table's `licence`).
internal data class AboutTerm(val title: String, val text: String)

internal data class AboutLicence(
    val title: String? = null,
    val spdx: String? = null,
    val copyright: List<String> = emptyList(),
    val statement: String? = null,
    val termsTitle: String? = null,
    val termsNote: String? = null,
    val terms: List<AboutTerm> = emptyList(),
    val freedoms: String? = null,
    val notices: String? = null
)

internal data class AboutInfo(
    val creditLines: List<String> = emptyList(),
    val makers: List<AboutMaker> = emptyList(),
    // WP CG29: the links section's heading and note, which say whose links they are.
    val linksTitle: String? = null,
    val linksNote: String? = null,
    val links: List<AboutLink> = emptyList(),
    val credits: List<AboutCredit> = emptyList(),
    val licence: AboutLicence = AboutLicence()
)

/**
 * Parses `aboutJson()`, keeping the file's order. Text that is not the table gives an empty
 * [AboutInfo]; an entry missing its text is left out; fields this version doesn't know are ignored.
 */
internal fun parseAbout(json: String): AboutInfo {
    val root = Json.parseObject(json) ?: return AboutInfo()
    return AboutInfo(
        creditLines = root.entries("credit_lines").mapNotNull { it.text("text") },
        makers = root.entries("makers").mapNotNull { m ->
            m.text("name")?.let { name ->
                AboutMaker(name, m.text("url"), m.list("made").orEmpty().filterIsInstance<String>().filter { it.isNotBlank() })
            }
        },
        linksTitle = root.text("links_title"),
        linksNote = root.text("links_note"),
        links = root.entries("links").mapNotNull { l ->
            l.text("title")?.let { AboutLink(l.str("id").orEmpty(), it, l.text("detail"), l.text("url")) }
        },
        credits = root.entries("credits").mapNotNull { c ->
            c.text("name")?.let { AboutCredit(it, c.text("what"), c.text("url")) }
        },
        licence = parseLicence(root.obj("licence"))
    )
}

// -- WP LIC1 --
private fun parseLicence(l: Map<String, Any?>?): AboutLicence {
    if (l == null) return AboutLicence()
    return AboutLicence(
        title = l.text("title"),
        spdx = l.text("spdx"),
        copyright = l.list("copyright").orEmpty().filterIsInstance<String>().filter { it.isNotBlank() },
        statement = l.text("statement"),
        termsTitle = l.text("terms_title"),
        termsNote = l.text("terms_note"),
        terms = l.entries("terms").mapNotNull { t -> t.text("text")?.let { AboutTerm(t.text("title").orEmpty(), it) } },
        freedoms = l.text("freedoms"),
        notices = l.text("notices")
    )
}

@Suppress("UNCHECKED_CAST")
private fun Map<String, Any?>.obj(key: String): Map<String, Any?>? = this[key] as? Map<String, Any?>

private fun Map<String, Any?>.text(key: String): String? = str(key)?.takeIf { it.isNotBlank() }

@Suppress("UNCHECKED_CAST")
private fun Map<String, Any?>.entries(key: String): List<Map<String, Any?>> =
    list(key).orEmpty().filterIsInstance<Map<*, *>>().map { it as Map<String, Any?> }
