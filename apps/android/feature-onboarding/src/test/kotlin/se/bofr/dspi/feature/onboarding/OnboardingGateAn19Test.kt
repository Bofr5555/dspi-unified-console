package se.bofr.dspi.feature.onboarding

import org.junit.Assert.assertEquals
import org.junit.Test
import se.bofr.dspi.ui.settings.AppSettings

/** WP AN19: the launch decision and the unread notes, as pure functions. */
class OnboardingGateAn19Test {
    // "1.2.3" or "1.2.3-betaN"; anything else is not a version. A beta sorts below its release.
    private val order = ReleaseOrder { a, b ->
        fun key(v: String): List<Int>? {
            val m = Regex("""(\d+)\.(\d+)\.(\d+)(?:-beta(\d+))?""").matchEntire(v) ?: return null
            val (x, y, z, beta) = m.destructured
            return listOf(x.toInt(), y.toInt(), z.toInt(), if (beta.isEmpty()) 256 else beta.toInt())
        }
        val ka = key(a) ?: return@ReleaseOrder null
        val kb = key(b) ?: return@ReleaseOrder null
        ka.zip(kb).map { (p, q) -> p.compareTo(q) }.firstOrNull { it != 0 } ?: 0
    }

    private val notes = listOf("1.1.6", "1.1.6-beta5", "1.1.6-beta4", "1.1.5", "next").map { ReleaseNotes(it, "h$it", listOf("i")) }

    private fun launch(s: AppSettings, current: String? = "1.1.6-beta5") = OnboardingGate.atLaunch(s, notes, current, order)

    @Test
    fun wpAn19_newInstallGetsTheWizardAndStartsWithTheNotesRead() {
        val l = launch(AppSettings())
        assertEquals(OnboardingGate.Show.GETTING_STARTED, l.show)
        assertEquals("1.1.6-beta5", l.update(AppSettings()).whatsNewLastShownVersion)
        assertEquals("setup is done only on Skip or Finish", false, l.update(AppSettings()).onboardingSetupDone)
    }

    @Test
    fun wpAn19_wizardAgainKeepsTheNotesAlreadyRead() {
        val s = AppSettings(whatsNewLastShownVersion = "1.1.5")
        val l = launch(s)
        assertEquals(OnboardingGate.Show.GETTING_STARTED, l.show)
        assertEquals("1.1.5", l.update(s).whatsNewLastShownVersion)
    }

    @Test
    fun wpAn19_userFromBeforeWhatsNewIsMarkedReadQuietly() {
        val s = AppSettings(onboardingSetupDone = true)
        val l = launch(s)
        assertEquals(OnboardingGate.Show.NOTHING, l.show)
        assertEquals("1.1.6-beta5", l.update(s).whatsNewLastShownVersion)
    }

    @Test
    fun wpAn19_updateShowsWhatsNewOnceThenNothing() {
        val s = AppSettings(onboardingSetupDone = true, whatsNewLastShownVersion = "1.1.6-beta4")
        val l = launch(s)
        assertEquals(OnboardingGate.Show.WHATS_NEW, l.show)
        val after = l.update(s)
        assertEquals("1.1.6-beta5", after.whatsNewLastShownVersion)
        assertEquals(OnboardingGate.Show.NOTHING, launch(after).show)
    }

    @Test
    fun wpAn19_notesAreBoundedByWhatWasShownAndThisBuild() {
        assertEquals(listOf("1.1.6-beta5", "1.1.6-beta4"), OnboardingGate.unread(notes, "1.1.5", "1.1.6-beta5", order).map { it.version })
        assertEquals(
            "nothing past this build",
            emptyList<ReleaseNotes>(),
            OnboardingGate.unread(notes, "1.1.6-beta5", "1.1.6-beta5", order)
        )
        assertEquals("unknown build: nothing", emptyList<ReleaseNotes>(), OnboardingGate.unread(notes, "1.1.5", null, order))
        assertEquals("unparseable shown: nothing", emptyList<ReleaseNotes>(), OnboardingGate.unread(notes, "garbage", "1.1.6", order))
        assertEquals(
            OnboardingGate.Show.NOTHING,
            launch(AppSettings(onboardingSetupDone = true, whatsNewLastShownVersion = "1.1.5"), null).show
        )
    }

    @Test
    fun wpAn19_newestFirstWhateverTheFileOrder() {
        val shuffled = listOf(notes[3], notes[4], notes[0], notes[2], notes[1])
        assertEquals(
            listOf("1.1.6", "1.1.6-beta5", "1.1.6-beta4", "1.1.5", "next"),
            OnboardingGate.newestFirst(shuffled, order).map { it.version }
        )
    }

    @Test
    fun wpAn19_boardWordsFollowTheMatch() {
        val stage = OnboardingStage(
            "board",
            "Board",
            cards = listOf(
                OnboardingCard("card.ready", "", "", "Install"),
                OnboardingCard("card.update_available", "", "", "Update"),
                OnboardingCard("card.downgrade", "", "", "Downgrade")
            )
        )
        assertEquals("board.prepare", BoardWords.headerId(BoardMatch.NONE))
        assertEquals("card.already_installed", BoardWords.cardId(BoardMatch.MATCH))
        assertEquals("Install", BoardWords.action(stage, BoardMatch.NONE))
        assertEquals("Update", BoardWords.action(stage, BoardMatch.DEVICE_OLDER))
        assertEquals("Downgrade", BoardWords.action(stage, BoardMatch.DEVICE_NEWER))
        assertEquals(null, BoardWords.action(stage, BoardMatch.MATCH))
    }
}
