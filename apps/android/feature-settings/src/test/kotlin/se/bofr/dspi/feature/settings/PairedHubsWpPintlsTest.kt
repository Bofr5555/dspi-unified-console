package se.bofr.dspi.feature.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.ui.link.PairedHubRow

/**
 * WP PINTLS (owner decision 2026-11-17): a paired hub's line says it is pinned, encrypted-only
 * after Pair Again, or unencrypted; the footer says Forget is the only way back to an
 * unencrypted pairing.
 */
class PairedHubsWpPintlsTest {
    @Test
    fun wpPintlsThePairedHubLineAndTheForgetWords() {
        assertEquals("Key SHA256:K7Q2", pairedHubDetail(PairedHubRow("h", "Studio", "SHA256:K7Q2", tlsOnly = true)))
        assertEquals("Encrypted only; pair again", pairedHubDetail(PairedHubRow("h", "Studio", "", tlsOnly = true)))
        assertEquals("Unencrypted", pairedHubDetail(PairedHubRow("h", "Studio")))
        assertTrue(PAIRED_HUBS_FOOTER, PAIRED_HUBS_FOOTER.contains("Forgetting a hub deletes its pairing and its key"))
        assertTrue(PAIRED_HUBS_FOOTER, PAIRED_HUBS_FOOTER.contains("paired unencrypted"))
        assertTrue(PAIRED_HUBS_FOOTER, PAIRED_HUBS_FOOTER.contains("keeps it encrypted-only"))
    }
}
