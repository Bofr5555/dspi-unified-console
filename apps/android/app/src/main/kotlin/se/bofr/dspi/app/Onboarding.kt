// WP AN19: Getting Started and What's New in the NavHost, and what to show at launch.
// Here rather than in :feature-onboarding because features do not depend on each
// other (PACKAGES.md): the firmware versions come from :feature-firmware.
package se.bofr.dspi.app

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavHostController
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.firmware.FirmwareMatch
import se.bofr.dspi.feature.firmware.FirmwareVersion
import se.bofr.dspi.feature.onboarding.BoardMatch
import se.bofr.dspi.feature.onboarding.BoardStatus
import se.bofr.dspi.feature.onboarding.GettingStartedScreen
import se.bofr.dspi.feature.onboarding.OnboardingGate
import se.bofr.dspi.feature.onboarding.OnboardingSource
import se.bofr.dspi.feature.onboarding.OnboardingText
import se.bofr.dspi.feature.onboarding.ReleaseOrder
import se.bofr.dspi.feature.onboarding.WhatsNewScreen
import se.bofr.dspi.feature.settings.SpeakerWords
import se.bofr.dspi.feature.settings.SpeakersEditor
import se.bofr.dspi.feature.settings.SpeakersModel
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.LocalDemoMode
import se.bofr.dspi.ui.settings.LocalAppSettings

/**
 * The core's onboarding data. Null (tests, previews): no launch decision, and the two
 * pages open empty.
 */
internal val LocalOnboarding = staticCompositionLocalOf<OnboardingSource?> { null }

/** Release notes are versioned as the Console, whose firmware this app ships: a beta sorts below its release. */
internal val FirmwareReleaseOrder = ReleaseOrder { a, b ->
    val x = FirmwareVersion.parse(a)
    val y = FirmwareVersion.parse(b)
    if (x == null || y == null) null else x.compareTo(y)
}

/** The release this build is, in the notes' spelling ("1.1.6-beta5"). */
internal val currentRelease: String? get() = FirmwareVersion.expected?.tagSuffix

/** The connected board against the firmware this app ships (macOS `firmwareMatch`). */
internal fun boardStatus(connected: Boolean, device: FirmwareVersion?): BoardStatus {
    val match = when (FirmwareMatch.of(connected, device)) {
        null -> BoardMatch.NONE
        FirmwareMatch.MATCH -> BoardMatch.MATCH
        FirmwareMatch.DEVICE_OLDER -> BoardMatch.DEVICE_OLDER
        FirmwareMatch.DEVICE_NEWER -> BoardMatch.DEVICE_NEWER
    }
    return BoardStatus(match, device?.description.orEmpty(), FirmwareVersion.expected?.description.orEmpty())
}

@Composable
internal fun GettingStartedRoute(bridge: DspiBridge, go: (String) -> Unit, back: () -> Unit) {
    val source = LocalOnboarding.current ?: OnboardingSource.EMPTY
    val words = remember(source) { OnboardingText.parseGettingStarted(source.gettingStartedJson()) }
    val store = LocalAppSettings.current
    val status by bridge.status.collectAsState()
    val state by bridge.state.collectAsState()
    val board = boardStatus(status.connected, if (status.connected) FirmwareVersion.of(state) else null)
    // -- WP UAN -- (UXP B8) the Speakers step, the Settings page's rows
    val scope = rememberCoroutineScope()
    val speakers = remember(bridge) { SpeakersModel(bridge, scope) }
    val speakersState by speakers.state.collectAsState()
    GettingStartedScreen(
        words,
        board,
        onInstall = { go(DspiRoutes.FIRMWARE) },
        onDone = {
            store.update { it.copy(onboardingSetupDone = true) }
            back()
        },
        speakersStage = {
            Text(SpeakerWords.TITLE, style = MaterialTheme.typography.headlineSmall)
            SpeakersEditor(bridge, speakers)
            if (!speakersState.confirmed) Text(SpeakerWords.LATER, style = MaterialTheme.typography.bodyMedium)
        },
        speakersConfirmed = speakersState.confirmed && !speakersState.changed
    )
}

@Composable
internal fun WhatsNewRoute(back: () -> Unit) {
    val source = LocalOnboarding.current ?: OnboardingSource.EMPTY
    val releases =
        remember(source) { OnboardingGate.newestFirst(OnboardingText.parseWhatsNew(source.whatsNewJson()), FirmwareReleaseOrder) }
    WhatsNewScreen(releases, onBack = back)
}

/**
 * Once the stored settings are in, opens Getting Started on a first run or What's New
 * after an update (OnboardingGate). Not in demo mode: the wizard is about a real board,
 * as macOS skips it on a demo launch.
 */
@Composable
internal fun OnboardingAtLaunch(nav: NavHostController) {
    val source = LocalOnboarding.current ?: return
    if (LocalDemoMode.current) return
    val store = LocalAppSettings.current
    val loaded by store.settingsLoaded.collectAsStateWithLifecycle()
    var decided by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(loaded) {
        if (!loaded || decided) return@LaunchedEffect
        decided = true
        val notes = OnboardingText.parseWhatsNew(source.whatsNewJson())
        val launch = OnboardingGate.atLaunch(store.settings.value, notes, currentRelease, FirmwareReleaseOrder)
        store.update(launch.update)
        when (launch.show) {
            OnboardingGate.Show.GETTING_STARTED -> nav.navigate(DspiRoutes.GETTING_STARTED) { launchSingleTop = true }
            OnboardingGate.Show.WHATS_NEW -> nav.navigate(DspiRoutes.WHATS_NEW) { launchSingleTop = true }
            OnboardingGate.Show.NOTHING -> Unit
        }
    }
}
