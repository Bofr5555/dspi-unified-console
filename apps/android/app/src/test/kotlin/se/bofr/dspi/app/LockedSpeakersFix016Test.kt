package se.bofr.dspi.app

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.onboarding.CONTINUE_TAG
import se.bofr.dspi.feature.onboarding.OnboardingSource
import se.bofr.dspi.feature.settings.SPEAKERS_CONFIRM_TAG
import se.bofr.dspi.feature.settings.SPEAKERS_PAGE_TAG
import se.bofr.dspi.feature.settings.SettingsCategory
import se.bofr.dspi.feature.settings.SettingsView
import se.bofr.dspi.feature.settings.SpeakerWords
import se.bofr.dspi.feature.settings.speakerRoleTag
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.LocalUiMode
import se.bofr.dspi.ui.LocalWritesAllowed
import se.bofr.dspi.ui.UiModes
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * -- FIX016 -- (REVG R4, agent C F2 and F7) the speaker layout is the installer's under the lock: the roles the
 * preset guard holds a tweeter by. Getting Started, a route the locked user still has, shows no Speakers step;
 * unlocked it does. Settings › Speakers is under the write gate: a read-only session sees the rows and can change
 * none of them, nor Confirm. Over the real core's demo device.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h3000dp-port-mdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class LockedSpeakersFix016Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val core = FfiDspiCore()
    private val source = OnboardingSource(core::onboardingWhatsNewJson, core::onboardingGettingStartedJson)
    private val settings = InMemoryAppSettingsStore(AppSettings())

    @After
    fun stop() {
        rule.runOnUiThread { session?.close() }
        scope.cancel()
    }

    private fun pumpUntil(what: String, until: () -> Boolean) {
        val end = System.currentTimeMillis() + 20_000
        while (!until()) {
            if (System.currentTimeMillis() > end) throw AssertionError("timed out: $what")
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    private fun settle() {
        repeat(6) {
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
    }

    private fun shown(tag: String) = rule.onAllNodes(hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun textShown(text: String) =
        rule.onAllNodes(hasText(text, substring = true), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun demo(): DspiBridge {
        val bridge = startDemoSession(scope, "fix016-locked").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        return bridge
    }

    /** The installer's lock on the demo, as ModeUiWpP1an2Test sets it. */
    private fun lock(bridge: DspiBridge) {
        var done = false
        rule.runOnUiThread {
            scope.launch {
                bridge.installer.setPin("2468").getOrThrow()
                bridge.installer.cover(listOf("front")).getOrThrow()
                bridge.installer.lockNow().getOrThrow()
                done = true
            }
        }
        pumpUntil("locked") { done && bridge.installer.lock.value?.locked == true }
    }

    /** Walks Getting Started to its last step; whether the Speakers editor was on any step. */
    private fun walkGettingStarted(): Boolean {
        var speakers = false
        repeat(12) {
            settle()
            speakers = speakers || shown(speakerRoleTag(0)) || textShown(SpeakerWords.TITLE)
            if (!shown(CONTINUE_TAG)) return speakers
            rule.onNodeWithTag(CONTINUE_TAG).performClick()
        }
        throw AssertionError("Getting Started never reached its last step")
    }

    private fun gettingStarted(bridge: DspiBridge, mode: String) {
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides settings, LocalOnboarding provides source, LocalUiMode provides mode) {
                DspiTheme(darkTheme = true) { GettingStartedRoute(bridge, {}, {}) }
            }
        }
        pumpUntil("writes") { bridge.writesAllowed.value }
    }

    @Test
    fun fix016LockedGettingStartedHasNoSpeakersStep() {
        val bridge = demo()
        gettingStarted(bridge, UiModes.LOCKED_USER)
        lock(bridge)
        assertFalse("no Speakers step under the lock", walkGettingStarted())
    }

    @Test
    fun fix016UnlockedGettingStartedKeepsTheSpeakersStep() {
        val bridge = demo()
        gettingStarted(bridge, UiModes.ADVANCED)
        assertTrue("the Speakers step, unlocked", walkGettingStarted())
    }

    @Test
    fun fix016ReadOnlySettingsSpeakersChangesNothing() {
        val bridge = demo()
        rule.setContent {
            CompositionLocalProvider(LocalAppSettings provides settings, LocalWritesAllowed provides false) {
                DspiTheme(darkTheme = true) { SettingsView(bridge, settings, {}, {}, initialPage = SettingsCategory.SPEAKERS) }
            }
        }
        pumpUntil("page") { shown(SPEAKERS_PAGE_TAG) }
        pumpUntil("rows") { shown(speakerRoleTag(0)) }
        for (o in 0 until bridge.numOutputChannels) {
            assertFalse(
                "Out ${o + 1}'s role is not offered read-only",
                rule.onAllNodes(hasTestTag(speakerRoleTag(o)) and isEnabled(), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
            )
        }
        assertFalse(
            "no Confirm read-only",
            rule.onAllNodes(hasTestTag(SPEAKERS_CONFIRM_TAG) and isEnabled(), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        )
        assertFalse(
            "no Suggestion read-only",
            rule.onAllNodes(hasText("Use the Suggestion") and isEnabled()).fetchSemanticsNodes().isNotEmpty()
        )
    }
}
