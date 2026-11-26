package se.bofr.dspi.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.MeasureRole
import se.bofr.dspi.core.SpeakerLayout
import se.bofr.dspi.feature.onboarding.GettingStartedScreen
import se.bofr.dspi.feature.onboarding.OnboardingText
import se.bofr.dspi.feature.onboarding.SPEAKERS_LATER
import se.bofr.dspi.feature.settings.SPEAKERS_CONFIRM_TAG
import se.bofr.dspi.feature.settings.SpeakerWords
import se.bofr.dspi.feature.settings.SpeakersSettingsTab
import se.bofr.dspi.feature.settings.speakerRoleTag
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.settings.AppSettings
import se.bofr.dspi.ui.settings.InMemoryAppSettingsStore
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * WP UAN (UXP B8) over the real core's demo device: Settings › Speakers shows every output, a
 * tweeter row says its high-pass floor, and Confirm keeps the user's roles in the core (the
 * layout the measurement and the preset guard trust); Getting Started has the step before Done,
 * Later by default. With `-Pdspi.shotsDir=<dir>` the page is also saved as a screenshot.
 */
@RunWith(DspiRobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h2000dp-port-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SpeakersPageWpUanTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: DemoSession? = null
    private val dir = System.getProperty("dspi.shotsDir")

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

    private fun textShown(text: String) =
        rule.onAllNodes(hasText(text, substring = true), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()

    private fun shoot(name: String) {
        if (dir.isNullOrEmpty()) return
        rule.mainClock.advanceTimeBy(400)
        shadowOf(Looper.getMainLooper()).idle()
        val out = File(dir).apply { mkdirs() }.resolve("$name.png")
        rule.runOnUiThread {
            val root = rule.activity.window.decorView
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))
            out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    private fun demo(): DspiBridge {
        val bridge = startDemoSession(scope, "uan-speakers").also { session = it }.bridge
        rule.mainClock.autoAdvance = false
        return bridge
    }

    private fun kept(bridge: DspiBridge): SpeakerLayout? {
        var l: SpeakerLayout? = null
        var done = false
        rule.runOnUiThread {
            scope.launch {
                l = bridge.speakers.current().getOrNull()
                done = true
            }
        }
        pumpUntil("read") { done }
        return l
    }

    @Test
    fun uanSettingsSpeakersKeepsTheUsersRoles() {
        val bridge = demo()
        val settings = InMemoryAppSettingsStore(AppSettings())
        rule.setContent { CompositionLocalProvider(LocalAppSettings provides settings) { DspiTheme(darkTheme = true) { SpeakersSettingsTab(bridge) } } }
        pumpUntil("writes") { bridge.writesAllowed.value }
        pumpUntil("rows") { rule.onAllNodes(hasTestTag(speakerRoleTag(0))).fetchSemanticsNodes().isNotEmpty() }
        assertTrue(textShown(SpeakerWords.UNCONFIRMED))
        assertEquals("a row per output", bridge.numOutputChannels, (0 until 16).count { o -> rule.onAllNodes(hasTestTag(speakerRoleTag(o))).fetchSemanticsNodes().isNotEmpty() })
        // Out 2 is a tweeter: its row says its floor
        rule.onNodeWithTag(speakerRoleTag(1)).performClick()
        pumpUntil("menu") { textShown("Tweeter") }
        rule.onNode(hasText("Tweeter") and hasClickAction()).performClick()
        pumpUntil("floor") { textShown("Its high-pass: at least 1000 Hz, 12 dB/octave") }
        shoot("uan-speakers-page-dark")
        rule.onNodeWithTag(SPEAKERS_CONFIRM_TAG).performClick()
        pumpUntil("confirmed") { textShown(SpeakerWords.CONFIRMED) }
        val l = kept(bridge)!!
        assertTrue("the user's word", l.confirmedByUser)
        assertEquals(MeasureRole.HIGH, l.outputs.first { it.output == 1 }.role)
    }

    @Test
    fun uanGettingStartedHasTheSpeakersStepLaterByDefault() {
        val words = OnboardingText.parseGettingStarted(FfiDspiCore().onboardingGettingStartedJson())
        rule.setContent {
            DspiTheme {
                GettingStartedScreen(words, boardStatus(false, null), onInstall = {}, onDone = {}, speakersStage = {
                    androidx.compose.material3.Text(SpeakerWords.TITLE)
                })
            }
        }
        val continueWord = words.button("continue")
        rule.onNode(hasText(continueWord) and hasClickAction()).performClick() // to the board
        rule.onNode(hasText(continueWord) and hasClickAction()).performClick() // to the speakers
        rule.onNode(hasText(SpeakerWords.TITLE)).assertExists()
        rule.onNode(hasText(SPEAKERS_LATER) and hasClickAction()).performClick()
        rule.onNode(hasText(words.stage("done")!!.title)).assertExists()
    }
}
