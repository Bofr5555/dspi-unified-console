package se.bofr.dspi.feature.settings

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.str

/** AN26: the About page shows what `assets/about.json` holds, whatever it says. */
class AboutInfoAn26Test {
    private val asset = File("../../../assets/about.json").readText()

    @Suppress("UNCHECKED_CAST")
    private fun rawStrings(section: String, key: String): List<String?> =
        Json.parseObject(asset)!!.list(section)!!.map { (it as Map<String, Any?>).str(key) }

    @Test
    fun wpAn26HoldsEveryEntryOfTheFileInOrder() {
        val a = parseAbout(asset)
        assertEquals(rawStrings("credit_lines", "text"), a.creditLines)
        assertEquals(rawStrings("makers", "name"), a.makers.map { it.name })
        assertEquals(rawStrings("makers", "url"), a.makers.map { it.url })
        assertEquals(rawStrings("links", "id"), a.links.map { it.id })
        assertEquals(rawStrings("links", "title"), a.links.map { it.title })
        assertEquals(rawStrings("links", "detail"), a.links.map { it.detail })
        assertEquals(rawStrings("links", "url"), a.links.map { it.url })
        assertEquals(rawStrings("credits", "name"), a.credits.map { it.name })
        assertEquals(rawStrings("credits", "what"), a.credits.map { it.what })
        assertEquals(rawStrings("credits", "url"), a.credits.map { it.url })
        assertTrue(a.makers.all { it.made.isNotEmpty() })
        // -- WP LIC1 --
        @Suppress("UNCHECKED_CAST")
        val raw = Json.parseObject(asset)!!["licence"] as Map<String, Any?>
        assertEquals(raw.str("statement"), a.licence.statement)
        assertEquals(raw.list("copyright"), a.licence.copyright)
        @Suppress("UNCHECKED_CAST")
        val terms = raw.list("terms")!!.map { it as Map<String, Any?> }
        assertEquals(terms.map { it.str("text") }, a.licence.terms.map { it.text })
        assertEquals(terms.map { it.str("title") }, a.licence.terms.map { it.title })
    }

    @Test
    fun wpCg29ReadsTheLinksTitleAndNote() {
        val a = parseAbout(asset)
        assertEquals("Weeb Labs: links and support", a.linksTitle)
        assertEquals(
            "Weeb Labs makes DSPi, its firmware and the original DSPi Consoles; these are their channels, " +
                "and Patreon and Ko-fi are where to support their work. This console is made by Bofr5555, not Weeb Labs.",
            a.linksNote
        )
        val words = (a.linksTitle + " " + a.linksNote).lowercase().split(Regex("[^a-z0-9']+"))
        for (host in listOf("phone", "tablet", "computer", "mac", "pc", "laptop")) {
            assertFalse(host, words.any { it.removeSuffix("'s") == host || it == host + "s" })
        }
        val none = parseAbout("""{"links":[{"id":"x","title":"T"}],"links_title":" "}""")
        assertEquals(null, none.linksTitle)
        assertEquals(null, none.linksNote)
    }

    @Test
    fun wpAn26TakesAnyWordingAndUnknownFields() {
        val a = parseAbout(
            """{"draft":false,"future":{"x":1},
               "credit_lines":[{"id":"a","text":"Någon annan rad"},{"id":"blank","text":" "},{"id":"none"}],
               "makers":[{"id":"m","name":"Ny","made":[]},{"id":"nameless","url":"https://x.example"}],
               "links":[{"id":"new_site","title":"Webb","url":"https://y.example","extra":true}],
               "credits":[{"id":"c","name":"Bara ett namn"}]}"""
        )
        assertEquals(listOf("Någon annan rad"), a.creditLines)
        assertEquals(listOf(AboutMaker("Ny", null, emptyList())), a.makers)
        assertEquals(listOf(AboutLink("new_site", "Webb", null, "https://y.example")), a.links)
        assertEquals(listOf(AboutCredit("Bara ett namn", null, null)), a.credits)
    }

    @Test
    fun wpAn26BrokenTextGivesAnEmptyTable() {
        for (bad in listOf("", "not json", "[1]", """{"makers":""", """{"makers":"x","links":[1,null],"credits":{}}""")) {
            assertEquals(bad, AboutInfo(), parseAbout(bad))
        }
    }

    @Test
    fun wpAn26ThePageCarriesNoCopyOfTheTable() {
        // CG29b (owner): the support paragraph names its speaker; that is not a copy of the table
        val page = File("src/main/kotlin/se/bofr/dspi/feature/settings/GeneralSettingsTab.kt").readText()
            .replace("that I (Weeb Labs) develop", "")
        for (word in listOf("Weeb Labs", "Bofr5555", "weeblabs", "discord.gg", "ko-fi.com")) {
            assertFalse(word, page.contains(word))
        }
    }
}
