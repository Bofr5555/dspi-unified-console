package se.bofr.dspi.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.filter
import se.bofr.dspi.bridge.BridgeEvent
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.feature.presets.DocumentOpener
import se.bofr.dspi.feature.presets.LocalDocumentOpener
import se.bofr.dspi.feature.settings.LocalAboutJson
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.LocalUserVolumeShown
import se.bofr.dspi.ui.UserVolumeShown
import se.bofr.dspi.ui.collectAsStateOnMain
import se.bofr.dspi.ui.link.LocalNetworking
import se.bofr.dspi.ui.settings.LocalAppSettings
import se.bofr.dspi.ui.state.LocalAppState

/**
 * The only Activity. Owns nothing: the stack lives in [DspiApplication].
 * - `USB_DEVICE_ATTACHED` launches (implicit permission, device_filter) are
 *   handed to the session in [onCreate] and [onNewIntent] (singleTask);
 * - API 33+: asks for POST_NOTIFICATIONS (see [shouldAskNotificationPermission]) so the
 *   connectedDevice foreground service's notification (volume/mute/preset)
 *   is visible (review U7). Denial is fine: the service still runs.
 */
class MainActivity : ComponentActivity() {
    private val app get() = application as DspiApplication

    // -- WP SECAN -- a `dspi://pair` link waiting for the user's confirmation (or why it is not one)
    private var pairLink by mutableStateOf<LinkPairLink?>(null)
    private var badPairLink by mutableStateOf<String?>(null)

    // -- WP ANVOL -- the volume buttons on the current bridge's User Volume
    private val userVolumeShown = UserVolumeShown()
    private val volumeKeys by lazy {
        val host = BridgeVolumeKeyHost(
            bridge = { app.modes.bridge.value },
            setting = { app.appSettings.settings.value.volumeKeysControlUserVolume },
            resumed = { lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) }
        )
        VolumeKeyController(app.appScope, host, SystemClock::uptimeMillis)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean = volumeKeys.onKey(event) || super.dispatchKeyEvent(event)

    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { /* nothing to do either way */ }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // a recreated Activity (rotation) still carries the launch intent: hand it over once
        if (savedInstanceState == null) {
            handAttachedIntent(intent)
            handPairLink(intent) // -- WP SECAN --
            requestNotificationPermission()
        }
        setContent {
            DspiTheme {
                val modes = app.modes
                val bridge by modes.bridge.collectAsStateWithLifecycle()
                val mode by modes.mode.collectAsStateWithLifecycle()
                val usbState by app.session.state.collectAsStateWithLifecycle()
                val remote by modes.remoteState.collectAsStateWithLifecycle()
                val usbPrompt by modes.usbPrompt.collectAsStateWithLifecycle()
                val link by app.linkSettings.settings.collectAsStateWithLifecycle()
                val hubs by app.networking.hubs.collectAsStateWithLifecycle() // -- WP AN7 --
                var networkingOpen by remember { mutableStateOf(false) }
                val document by bridge.snapshots.offline.collectAsStateWithLifecycle() // -- WP P1AN1 --
                val security by modes.remoteSecurity.collectAsStateOnMain() // -- WP SECAN --
                // macOS DeviceLockNotice: "Another console is applying changes" for 4 s after a LOCKED refusal
                var lockNotice by remember { mutableStateOf(false) }
                LaunchedEffect(bridge) {
                    bridge.events.filter { it is BridgeEvent.CommandFailed && it.error is DspiException.Locked }.collectLatest {
                        lockNotice = true
                        delay(4_000)
                        lockNotice = false
                    }
                }
                CompositionLocalProvider(
                    LocalUserVolumeShown provides userVolumeShown, // -- WP ANVOL --
                    LocalAppSettings provides app.appSettings,
                    LocalAppState provides app.appState,
                    LocalNetworking provides app.networking,
                    LocalAboutJson provides app.aboutJson,
                    LocalDocumentOpener provides DocumentOpener(modes::openDocument) // -- WP P1AN1 --
                ) {
                    AppScaffold(
                        bridge = bridge,
                        // -- WP P1AN1 --, SECAN: the security
                        strip = connectionStrip(mode, usbState, remote, lockNotice, document = document?.fileName, security = security),
                        // -- WP AN7 -- every user switch goes through the Unsaved Changes flow
                        onExitDemo = { modes.switchTo(DeviceChoice.Usb) },
                        onRequestPermission = { app.session.requestPermission(this) },
                        bootloaderLocator = app.bootloaderLocator,
                        onRemoteAction = { a ->
                            when (a) {
                                RemoteAction.RETRY -> modes.remoteSession?.retryNow()
                                RemoteAction.CONNECT -> remote.hub?.let { modes.remoteSession?.connect(it) }
                                RemoteAction.DISCONNECT -> modes.switchTo(DeviceChoice.Usb) // -- WP AN7 --
                            }
                        },
                        onConnectNetwork = { networkingOpen = true },
                        onboarding = app.onboarding, // -- WP AN19 --
                        // -- WP AN7 --
                        devices = deviceList(mode, usbState, remote, hubs, link.enabled),
                        onChooseDevice = modes::switchTo,
                        switchAlerts = modes.switchAlerts,
                        modeStore = app.uiModes, // -- WP P1AN2 --
                        overlay = {
                            RemoteDialogs(
                                state = remote,
                                usbPrompt = usbPrompt,
                                onPin = { modes.remoteSession?.submitPin(it) },
                                onCancelPin = { modes.remoteSession?.cancelPin() },
                                onChoose = { modes.remoteSession?.chooseDevice(it) },
                                onCancelChoice = { modes.exitRemote() },
                                onUsbAnswer = { modes.answerUsbPrompt(it) },
                                // -- WP SECAN --
                                security = security,
                                onRePair = { modes.remoteSession?.rePair() },
                                onKeyNoticeSeen = { modes.remoteSession?.ackKeyNotice() }
                            )
                            // -- WP SECAN -- a pairing link: one tap before anything is dialled
                            pairLink?.let { l ->
                                PairLinkDialog(
                                    l,
                                    link.enabled,
                                    onContinue = {
                                        pairLink = null
                                        if (!link.enabled) app.networking.setEnabled(true)
                                        modes.pairByLink(l)
                                    },
                                    onDismiss = { pairLink = null }
                                )
                            }
                            badPairLink?.let { r -> BadPairLinkDialog(r) { badPairLink = null } }
                            if (networkingOpen) NetworkingDialog(app.networking) { networkingOpen = false }
                            // -- WP ANVOL -- the volume buttons' level when the Home slider is not on screen
                            val volumeFeedback by volumeKeys.feedback.collectAsStateWithLifecycle()
                            val sliders by userVolumeShown.count.collectAsStateWithLifecycle()
                            VolumeKeyIndicator(volumeFeedback, sliderShown = sliders > 0)
                        }
                    )
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handAttachedIntent(intent)
        handPairLink(intent) // -- WP SECAN --
    }

    // -- WP SECAN --
    /**
     * A `dspi://pair` link (a scanner app's intent; any app can send one): read by the core
     * ([readPairLink]) and only offered, never acted on without the user's tap.
     */
    private fun handPairLink(intent: Intent?) {
        when (val r = readPairLink(intent?.action, intent?.dataString, app.parsePairLink)) {
            null -> Unit
            else -> r.fold(onSuccess = {
                badPairLink = null
                pairLink = it
            }, onFailure = { badPairLink = it.message ?: "not a valid pairing link" })
        }
    }

    /** Via the app's gate: after the links are loaded (review A3); application context, the Activity may be gone by then. */
    private fun handAttachedIntent(intent: Intent) {
        val ctx = applicationContext
        app.usbGate.run { app.session.onAttachedIntent(ctx, intent) }
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val prefs = getSharedPreferences(UI_PREFS, MODE_PRIVATE)
        val ask = shouldAskNotificationPermission(
            granted = ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED,
            askedBefore = prefs.getBoolean(KEY_NOTIF_ASKED, false),
            rationale = shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS)
        )
        if (!ask) return
        prefs.edit().putBoolean(KEY_NOTIF_ASKED, true).apply()
        notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    private companion object {
        const val UI_PREFS = "dspi_ui"
        const val KEY_NOTIF_ASKED = "notificationPermissionAsked"
    }
}

/**
 * Review A4: ask on the first launch, and again only while Android would
 * still show its dialog (`shouldShowRequestPermissionRationale` is true
 * after one denial, false after "don't ask again"). Never after a permanent
 * denial: `launch` would be a silent no-op on every start.
 */
fun shouldAskNotificationPermission(granted: Boolean, askedBefore: Boolean, rationale: Boolean): Boolean =
    !granted && (!askedBefore || rationale)

// -- WP SECAN --
/**
 * The pairing link in a VIEW intent, by the core's parser (`link_parse_pair_link`), or
 * null when the intent is no `dspi:` link at all. [parse] is the core's, swapped in tests.
 */
fun readPairLink(action: String?, data: String?, parse: (String) -> LinkPairLink): Result<LinkPairLink>? {
    if (action != Intent.ACTION_VIEW || data == null || !data.startsWith("dspi:", ignoreCase = true)) return null
    return try {
        Result.success(parse(data))
    } catch (e: DspiException) {
        Result.failure(IllegalArgumentException((e as? DspiException.InvalidArgument)?.msg ?: e.message.orEmpty()))
    }
}
