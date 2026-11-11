package se.bofr.dspi.feature.settings

import java.io.File
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Every licence text the Licences screen opens ships in assets/licenses/ (NOTICE.md). */
class LicensesTest {
    private val dir = File("src/main/assets/licenses")

    @Test
    fun everyReferencedTextIsBundled() {
        (THIRD_PARTY.mapNotNull { it.text } + GPL3).distinct().forEach {
            val f = File(dir, it.asset)
            assertTrue("missing ${f.path}", f.isFile && f.length() > 500)
        }
    }

    @Test
    fun wpAn1RustNoticesListWhatTheLibraryLinks() {
        val t = File(dir, RUST_CRATES.asset).readText()
        assertTrue(t.contains("\nuniffi "))
        assertTrue(t.contains("\nnusb "))
        assertTrue(t.contains("\ntungstenite "))
        // Gone with the old core, and what only the desktop apps link.
        assertFalse(t.contains("libusb"))
        assertFalse(t.contains("\nmdns-sd "))
        assertFalse(t.contains("\nmidir "))
    }

    /** WP LIC1: the terms the screen shows are the About table's; the first keeps both authors' attributions. */
    @Test
    fun aboutCreditsBothAuthors() {
        val licence = parseAbout(File("../../../assets/about.json").readText()).licence
        val first = licence.terms.first().text
        assertTrue(first.contains("Weeb Labs") && first.contains("Bofr5555"))
    }

    @Test
    fun wpLic1TheLicenceIsGpl3WithThreeTerms() {
        val licence = parseAbout(File("../../../assets/about.json").readText()).licence
        // WP CLEAN1: the program as a whole is GPL-3.0; Bofr5555's own parts are "or later"
        assertTrue(licence.spdx == "GPL-3.0-only")
        assertTrue(licence.statement!!.contains("either version 3 of the License, or (at your option) any later version"))
        val titles = listOf("Attributions (7b)", "Modified versions are marked (7c)", "No trademark rights (7e)")
        assertTrue(licence.terms.map { it.title } == titles)
        assertTrue(licence.copyright.size == 2 && licence.freedoms != null && licence.notices != null)
    }
}
