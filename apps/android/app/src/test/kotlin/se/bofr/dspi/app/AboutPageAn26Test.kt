package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertHasNoClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import se.bofr.dspi.feature.settings.GeneralSettingsTab
import se.bofr.dspi.feature.settings.LocalAboutJson
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.str

/** AN26: the About page over the CG15 table. Tall, so the lazy list composes every section. */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h5000dp-port-mdpi")
class AboutPageAn26Test {
    @get:Rule val rule = createComposeRule()

    private val opened = mutableListOf<String>()

    private fun show(json: String) {
        val uris = object : UriHandler {
            override fun openUri(uri: String) {
                opened += uri
            }
        }
        rule.setContent {
            CompositionLocalProvider(LocalAboutJson provides { json }, LocalUriHandler provides uris) {
                DspiTheme { GeneralSettingsTab() }
            }
        }
    }

    private fun assertGoogleRowsStay() {
        rule.onNodeWithText("Source code").assertHasClickAction()
        rule.onNodeWithText("https://github.com/Bofr5555/dspi-unified-console").assertExists()
        rule.onNodeWithText("Privacy policy").assertHasClickAction()
        rule.onNodeWithText("No data collected; no network access").assertExists()
        rule.onNodeWithText("Licences").assertHasClickAction()
    }

    @Test
    @Suppress("UNCHECKED_CAST")
    fun showsEverythingTheFileHolds() {
        val asset = File("../../../assets/about.json").readText()
        show(asset)
        val root = Json.parseObject(asset)!!
        fun strings(section: String, key: String) = root.list(section)!!.mapNotNull { (it as Map<String, Any?>).str(key) }
        val shown = strings("credit_lines", "text") + strings("makers", "name") +
            listOfNotNull(root.str("links_title"), root.str("links_note")) + strings("links", "title") +
            strings("links", "detail") + strings("credits", "name") + strings("credits", "what")
        assertTrue(shown.size > 20)
        // Some credits share their words, so count: one row per entry of the file.
        shown.groupingBy { it }.eachCount().forEach { (text, n) -> rule.onAllNodesWithText(text).assertCountEquals(n) }
        root.list("makers")!!.forEach { m ->
            val made = (m as Map<String, Any?>).list("made")!!.joinToString(", ")
            rule.onNodeWithText(made).assertExists()
        }
        assertGoogleRowsStay()
    }

    @Test
    fun wpCg29TheLinksSectionIsTitledAndNotedAsWeebLabs() {
        show(File("../../../assets/about.json").readText())
        rule.onNodeWithText("Weeb Labs: links and support").assertExists()
        rule.onNodeWithText(
            "Weeb Labs makes DSPi, its firmware and the original DSPi Consoles; these are their channels, " +
                "and Patreon and Ko-fi are where to support their work. This console is made by Bofr5555, not Weeb Labs."
        ).assertExists()
        rule.onNode(hasText("Links & Support")).assertDoesNotExist()
    }

    @Test
    fun anyWordingWorksAndARowOpensItsOwnUrl() {
        show(
            """{"draft":false,"credit_lines":[{"id":"x","text":"Annan rad"}],
               "makers":[{"id":"m","name":"Ny skapare","url":"https://m.example","made":["Allt"]}],
               "links":[{"id":"unknown_id","title":"Webbplats","url":"https://l.example"}],
               "credits":[{"id":"c","name":"Utan länk","what":"Något"},{"id":"d","name":"Med länk","url":"https://c.example"}]}"""
        )
        rule.onNodeWithText("Annan rad").assertExists()
        rule.onNodeWithText("Ny skapare").performClick()
        rule.onNodeWithText("Webbplats").performClick()
        rule.onNodeWithText("Med länk").performClick()
        rule.onNodeWithText("Utan länk").assertHasNoClickAction()
        assertEquals(listOf("https://m.example", "https://l.example", "https://c.example"), opened)
        assertGoogleRowsStay()
    }

    /** WP LIC1: About › Licences shows the table's licence notice and its three section 7 terms. */
    @Test
    @Suppress("UNCHECKED_CAST")
    fun wpLic1TheLicencesScreenShowsTheNoticeAndTheTerms() {
        val asset = File("../../../assets/about.json").readText()
        show(asset)
        val licence = Json.parseObject(asset)!!["licence"] as Map<String, Any?>
        rule.onNodeWithText("GPL-3.0-only with additional terms · third-party notices").assertExists()
        rule.onNodeWithText("Licences").performClick()
        rule.onNodeWithText(licence.str("statement")!!).assertExists()
        rule.onNodeWithText(licence.str("terms_title")!!).assertExists()
        rule.onNodeWithText(licence.str("terms_note")!!).assertExists()
        val terms = licence.list("terms")!!.map { it as Map<String, Any?> }
        assertEquals(3, terms.size)
        terms.forEachIndexed { i, t -> rule.onNodeWithText("${i + 1}. ${t.str("title")}. ${t.str("text")}").assertExists() }
        rule.onNodeWithText(licence.str("freedoms")!!).assertExists()
        rule.onNodeWithText((licence.list("copyright")!! as List<String>).joinToString("\n")).assertExists()
        rule.onNodeWithText("GNU General Public License v3.0").assertHasClickAction()
    }

    @Test
    fun aMissingTableLeavesTheRestOfThePage() {
        show("")
        rule.onNodeWithText("DSPi Console").assertExists()
        rule.onNode(hasText("Links & Support")).assertDoesNotExist()
        rule.onNode(hasText("Made by")).assertDoesNotExist()
        rule.onNode(hasText("Credits")).assertDoesNotExist()
        assertGoogleRowsStay()
    }
}
