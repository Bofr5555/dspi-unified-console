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

internal data class AboutInfo(
    val creditLines: List<String> = emptyList(),
    val makers: List<AboutMaker> = emptyList(),
    val links: List<AboutLink> = emptyList(),
    val credits: List<AboutCredit> = emptyList()
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
        links = root.entries("links").mapNotNull { l ->
            l.text("title")?.let { AboutLink(l.str("id").orEmpty(), it, l.text("detail"), l.text("url")) }
        },
        credits = root.entries("credits").mapNotNull { c ->
            c.text("name")?.let { AboutCredit(it, c.text("what"), c.text("url")) }
        }
    )
}

private fun Map<String, Any?>.text(key: String): String? = str(key)?.takeIf { it.isNotBlank() }

@Suppress("UNCHECKED_CAST")
private fun Map<String, Any?>.entries(key: String): List<Map<String, Any?>> =
    list(key).orEmpty().filterIsInstance<Map<*, *>>().map { it as Map<String, Any?> }
