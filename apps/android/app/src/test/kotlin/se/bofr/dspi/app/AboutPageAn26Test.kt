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
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import se.bofr.dspi.feature.settings.GeneralSettingsTab
import se.bofr.dspi.feature.settings.LocalAboutJson
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.json.Json
import se.bofr.dspi.ui.json.list
import se.bofr.dspi.ui.json.str

/** AN26: the About page over the CG15 table. Tall, so the lazy list composes every section. */
@RunWith(RobolectricTestRunner::class)
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
        rule.onNodeWithText("https://github.com/Bofr5555/dspi-android").assertExists()
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
            "Weeb Labs' channels for DSPi, its firmware and the original DSPi Consoles. " +
                "This console is made by Bofr5555, not Weeb Labs."
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
