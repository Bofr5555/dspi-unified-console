package se.bofr.dspi.bridge

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.core.BundledFirmware
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.fake.FakeDspiCore

/**
 * WP SECFIX: the "Not a verified DSPi Console image" label (SECFWV) comes from the core: its
 * manifest decides (`verify_bundled`, by the file's name), its notice row gives the words.
 */
class FirmwareLabelWpSecfixTest {
    private class ManifestCore(private val answer: BundledFirmware) : FakeDspiCore() {
        val asked = mutableListOf<String>()

        override fun verifyBundled(image: List<Int>, name: String): BundledFirmware {
            asked += name
            return answer
        }

        override fun noticeForKey(key: String, detail: String) = Notice(
            key,
            NoticeKind.INFO,
            "Not a verified DSPi Console image",
            "Not one of the app's images.",
            "Check where it came from.",
            detail
        )
    }

    @Test
    fun wpSecfixAPickedFileIsLabelledInTheCoresWords() = runTest {
        val core = ManifestCore(BundledFirmware.NOT_BUNDLED)
        val label = newBridge(core).coreTools.firmwareLabel(byteArrayOf(1, 2, 3), "mine.uf2")
        assertEquals(listOf("mine.uf2"), core.asked)
        assertEquals(FirmwareLabel("Not a verified DSPi Console image", "Not one of the app's images. Check where it came from."), label)
        // A bundled name with other bytes is not verified either
        assertEquals(
            label,
            newBridge(ManifestCore(BundledFirmware.MISMATCH)).coreTools.firmwareLabel(byteArrayOf(1), "DSPi-RP2350-v1.1.6.uf2")
        )
    }

    @Test
    fun wpSecfixAVerifiedCopyHasNoLabel() = runTest {
        assertNull(newBridge(ManifestCore(BundledFirmware.VERIFIED)).coreTools.firmwareLabel(byteArrayOf(1), "DSPi-RP2350-v1.1.6.uf2"))
    }

    @Test
    fun wpSecfixWithoutTheCoreNothingIsVerified() = runTest {
        // FakeDspiCore does not implement the manifest: the call fails, and the label shows
        assertEquals(FirmwareLabel.FALLBACK, newBridge(FakeDspiCore()).coreTools.firmwareLabel(byteArrayOf(1), "x.uf2"))
        assertEquals(FirmwareLabel.FALLBACK, CoreTools.Unavailable.firmwareLabel(byteArrayOf(1), "x.uf2"))
    }
}
