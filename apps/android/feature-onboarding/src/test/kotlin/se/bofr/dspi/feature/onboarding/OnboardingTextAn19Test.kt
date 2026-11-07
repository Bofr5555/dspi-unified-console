package se.bofr.dspi.feature.onboarding

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** WP AN19: reading the core's onboarding JSON (shapes from dspi_core::link_text). */
class OnboardingTextAn19Test {
    @Test
    fun wpAn19_whatsNewReadsReleasesAndSkipsWhatIsMalformed() {
        val json = """[{"version":"1.1.6","headline":"H","items":["a",{"text":"b"},3]},{"headline":"no version"},7]"""
        assertEquals(listOf(ReleaseNotes("1.1.6", "H", listOf("a", "b"))), OnboardingText.parseWhatsNew(json))
        assertEquals(emptyList<ReleaseNotes>(), OnboardingText.parseWhatsNew("not json"))
        assertEquals(emptyList<ReleaseNotes>(), OnboardingText.parseWhatsNew("{}"))
    }

    @Test
    fun wpAn19_gettingStartedReadsStagesLinesHeadersAndCards() {
        val json = """
            {"title":"Getting Started","step_counter":"Step {current} of {total}",
             "buttons":{"skip":"Skip Setup","continue":"Continue"},
             "stages":[
               {"id":"welcome","label":"Welcome","title":"Hi",
                "paragraphs":[{"id":"welcome.intro","text":"P1"},"P2"],
                "rows":[{"id":"welcome.shape","icon":"sliders","text":"R1"}]},
               {"id":"board","label":"Board",
                "headers":[{"id":"board.prepare","title":"T","blurb":"B"}],
                "cards":[{"id":"card.ready","title":"{chip} ready","text":"x","action":"Install"}]},
               {"label":"no id"}
             ]}
        """.trimIndent()
        val w = OnboardingText.parseGettingStarted(json)
        assertEquals("Getting Started", w.title)
        assertEquals("Skip Setup", w.button("skip"))
        assertEquals("", w.button("finish"))
        assertEquals(listOf("welcome", "board"), w.stages.map { it.id })
        val welcome = w.stage("welcome")!!
        assertEquals(listOf("P1", "P2"), welcome.paragraphs.map { it.text })
        assertEquals("sliders", welcome.rows.single().icon)
        assertEquals("Install", w.stage("board")!!.card("card.ready")!!.action)
        assertEquals("B", w.stage("board")!!.header("board.prepare")!!.blurb)
        assertTrue(OnboardingText.parseGettingStarted("[]").stages.isEmpty())
    }

    @Test
    fun wpAn19_fillReplacesOnlyTheNamesGiven() {
        assertEquals("Step 2 of 3", OnboardingText.fill("Step {current} of {total}", mapOf("current" to "2", "total" to "3")))
        assertEquals("RP2350 ready {volume}", OnboardingText.fill("{chip} ready {volume}", mapOf("chip" to "RP2350")))
    }
}
