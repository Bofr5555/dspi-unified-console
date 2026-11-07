package se.bofr.dspi.app

import android.app.Application
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import se.bofr.dspi.feature.firmware.FirmwareVersion
import se.bofr.dspi.feature.onboarding.BoardStatus
import se.bofr.dspi.feature.onboarding.GETTING_STARTED_TAG
import se.bofr.dspi.feature.onboarding.GettingStartedScreen
import se.bofr.dspi.feature.onboarding.GettingStartedWords
import se.bofr.dspi.feature.onboarding.OnboardingSource
import se.bofr.dspi.feature.onboarding.OnboardingText
import se.bofr.dspi.feature.onboarding.WHATS_NEW_RELEASE_TAG
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * AN19: Getting Started on a first run, once; What's New once after an update; both again
 * from Settings > About > Help. The whole app over a real demo bridge with the real core's
 * onboarding data (CG10). The strip is not the demo one, so the launch decision runs.
 * A tall window, so nothing needs scrolling (performScrollTo hangs on the paused clock).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h3000dp-port-mdpi")
class OnboardingAn19Test {
    @get:Rule val rule = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val core = FfiDspiCore()
    private val source = OnboardingSource(core::onboardingWhatsNewJson, core::onboardingGettingStartedJson)

    /** Bumped to start the app over (a new launch) with the same stored settings. */
    private var launch by mutableIntStateOf(0)

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun start(store: InMemoryAppSettingsStore) {
        val bridge = startDemoSession(scope, "test").also { session = it }.bridge
        // On Home, Compose does not go idle with the clock on auto, so drive the clock
        rule.mainClock.autoAdvance = false
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides store) {
                key(launch) {
                    AppScaffold(
                        bridge,
                        ConnectionStrip.Usb,
                        onExitDemo = {},
                        onRequestPermission = {},
                        onboarding = source
                    )
                }
            }
        }
        settle()
    }

    private fun relaunch() {
        launch++
        settle()
    }

    private fun settle() {
        rule.runOnUiThread { Snapshot.sendApplyNotifications() }
        rule.mainClock.advanceTimeBy(3_000)
    }

    private fun count(text: String) = rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes().size

    private fun clickable(text: String): SemanticsNodeInteraction {
        val m = hasText(text) and hasClickAction()
        assertEquals("clickable nodes with text \"$text\"", 1, rule.onAllNodes(m).fetchSemanticsNodes().size)
        return rule.onNode(m)
    }

    private fun tap(node: SemanticsNodeInteraction) {
        node.assertIsDisplayed().performClick()
        settle()
    }

    private val wizardShown get() = rule.onAllNodes(hasTestTag(GETTING_STARTED_TAG)).fetchSemanticsNodes().size == 1

    private fun releaseShown(version: String) = rule.onAllNodes(hasTestTag(WHATS_NEW_RELEASE_TAG + version), useUnmergedTree = true)
        .fetchSemanticsNodes().isNotEmpty()

    private val current = FirmwareVersion.expected!!.tagSuffix!!

    @Test
    fun wpAn19_firstRunShowsGettingStartedOnceFromTheCoresWords() {
        val store = InMemoryAppSettingsStore()
        start(store)
        assertTrue("first run opens the wizard", wizardShown)
        val words = OnboardingText.parseGettingStarted(core.onboardingGettingStartedJson())
        assertEquals(listOf("welcome", "board", "done"), words.stages.map { it.id })
        assertEquals(1, count(words.stage("welcome")!!.title))
        assertEquals(1, count(words.title))
        // A new install starts with the notes read
        assertEquals(current, store.settings.value.whatsNewLastShownVersion)

        // Through the stages and out; the board stage's Continue waits for matching firmware
        tap(clickable(words.button("continue")))
        assertEquals(1, count(words.stage("board")!!.label))
        tap(clickable(words.button("back")))
        tap(clickable(words.button("skip")))
        assertFalse("skipped", wizardShown)
        assertTrue(store.settings.value.onboardingSetupDone)

        relaunch()
        assertFalse("shown once", wizardShown)
        assertFalse("nothing new either", releaseShown(current))
    }

    private fun showWizard(board: BoardStatus, onInstall: () -> Unit, onDone: () -> Unit): GettingStartedWords {
        val words = OnboardingText.parseGettingStarted(core.onboardingGettingStartedJson())
        rule.setContent { DspiTheme { GettingStartedScreen(words, board, onInstall, onDone) } }
        return words
    }

    @Test
    fun wpAn19_matchingFirmwareLetsTheBoardStagePassAndFinishEndsSetup() {
        var done = 0
        val words = showWizard(boardStatus(true, FirmwareVersion.expected), onInstall = {}, onDone = { done++ })
        val board = words.stage("board")!!
        rule.onNode(hasText(words.button("continue")) and hasClickAction()).performClick()
        rule.onNodeWithText(board.header("board.current")!!.title).assertExists()
        val installed = OnboardingText.fill(
            board.card("card.already_installed")!!.title,
            mapOf(
                "bundled_version" to FirmwareVersion.expected!!.description
            )
        )
        rule.onNodeWithText(installed).assertExists()
        rule.onNode(hasText(words.button("continue")) and hasClickAction()).assertIsEnabled().performClick()
        rule.onNodeWithText(words.stage("done")!!.title).assertExists()
        assertEquals("no Skip on the last stage", 0, rule.onAllNodesWithText(words.button("skip")).fetchSemanticsNodes().size)
        rule.onNodeWithText(words.button("finish")).performClick()
        assertEquals(1, done)
    }

    @Test
    fun wpAn19_withNoBoardTheStageOpensFirmwareUpdateAndWaits() {
        var installs = 0
        val words = showWizard(boardStatus(false, null), onInstall = { installs++ }, onDone = {})
        val board = words.stage("board")!!
        rule.onNode(hasText(words.button("continue")) and hasClickAction()).performClick()
        rule.onNodeWithText(board.header("board.prepare")!!.title).assertExists()
        rule.onNodeWithText(board.card("card.waiting")!!.title).assertExists()
        rule.onNode(hasText(words.button("continue")) and hasClickAction()).assertIsNotEnabled()
        rule.onNodeWithText(board.card("card.ready")!!.action!!).performClick()
        assertEquals(1, installs)
    }

    @Test
    fun wpAn19_anOlderBoardIsOfferedTheUpdateWithBothVersions() {
        val older = FirmwareVersion(1, 1, 5)
        val words = showWizard(boardStatus(true, older), onInstall = {}, onDone = {})
        val board = words.stage("board")!!
        rule.onNode(hasText(words.button("continue")) and hasClickAction()).performClick()
        rule.onNodeWithText(board.header("board.older")!!.title).assertExists()
        val text = OnboardingText.fill(
            board.card("card.update_available")!!.text,
            mapOf("device_version" to older.description, "bundled_version" to FirmwareVersion.expected!!.description)
        )
        assertFalse("filled in", text.contains("{"))
        rule.onNodeWithText(text).assertExists()
        rule.onNodeWithText(board.card("card.update_available")!!.action!!).assertExists()
    }

    @Test
    fun wpAn19_whatsNewShowsOnceAfterAnUpdate() {
        val store = InMemoryAppSettingsStore(AppSettings(onboardingSetupDone = true, whatsNewLastShownVersion = "1.1.6-beta4"))
        start(store)
        assertFalse(wizardShown)
        assertTrue("the release after the one last shown", releaseShown(current))
        assertEquals(current, store.settings.value.whatsNewLastShownVersion)
        tap(clickable("Done"))
        assertFalse(releaseShown(current))

        relaunch()
        assertFalse("shown once", releaseShown(current))
    }

    @Test
    fun wpAn19_helpReplaysGettingStartedAndOpensWhatsNew() {
        val store = InMemoryAppSettingsStore(AppSettings(onboardingSetupDone = true, whatsNewLastShownVersion = current))
        start(store)
        assertFalse(wizardShown)
        fun openHelp() {
            val settings = hasContentDescription("Settings") and hasClickAction()
            assertEquals(1, rule.onAllNodes(settings).fetchSemanticsNodes().size)
            tap(rule.onNode(settings))
            tap(clickable("About"))
            assertEquals("the Help section", 1, count("Help"))
        }
        openHelp()
        tap(clickable("Getting Started"))
        assertTrue("replayed", wizardShown)
        val words = OnboardingText.parseGettingStarted(core.onboardingGettingStartedJson())
        tap(clickable(words.button("skip")))
        assertFalse(wizardShown)

        // Back where Help was: the About page
        tap(clickable("What's New"))
        assertTrue(releaseShown(current))
        tap(clickable("Done"))
        assertFalse(releaseShown(current))
    }
}
