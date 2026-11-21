package se.bofr.dspi.feature.settings

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WP ANSEC (owner 2026-11-21): Settings › Networking no longer says all DSPi Link traffic is
 * unencrypted. Since SECAN, DSPi Link to a uConsole for DSPi hub is encrypted with the hub's key
 * pinned at pairing; to Weeb Labs' DSPi Console or a DSPi bridge it is not. "DSPi Link", never a
 * bare "link"; and the acknowledged hubs' words say how to undo it.
 */
class NetworkingWordsWpAnsecTest {
    @Test
    fun wpAnsecTheDspiLinkFooterSaysWhichHubsAreEncrypted() {
        val f = DSPI_LINK_FOOTER
        assertFalse(f, f.contains("traffic is not encrypted"))
        assertTrue(f, f.contains("DSPi Link connections to a uConsole for DSPi hub are encrypted, and pairing pins the hub's key."))
        assertTrue(f, f.contains("Weeb Labs' DSPi Console or to a DSPi bridge are not encrypted; use those only on networks you trust."))
        assertTrue(f, f.contains("VPN"))
        // "link" only ever as "DSPi Link"
        val bare = Regex("(?<!DSPi )\\b[Ll]inks?\\b").findAll(f).toList()
        assertTrue(bare.map { it.value }.toString(), bare.isEmpty())
        assertFalse("not a Play-facing noun", f.contains("phone"))
    }

    @Test
    fun wpAnsecTheAcknowledgedHubsSayHowToUndoIt() {
        assertTrue(ACKNOWLEDGED_HUBS_FOOTER, ACKNOWLEDGED_HUBS_FOOTER.contains("still says \"(unsecured)\""))
        assertTrue(ACKNOWLEDGED_HUBS_FOOTER, ACKNOWLEDGED_HUBS_FOOTER.contains("Forget highlights a hub again"))
        assertTrue(ACKNOWLEDGED_HUBS_FOOTER, ACKNOWLEDGED_HUBS_FOOTER.contains("another hub answering at its address"))
    }
}
