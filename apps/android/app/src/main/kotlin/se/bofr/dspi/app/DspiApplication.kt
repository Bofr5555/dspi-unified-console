package se.bofr.dspi.app

import android.app.Application
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.feature.onboarding.OnboardingSource
import se.bofr.dspi.ffi.FfiDspiCore
import se.bofr.dspi.link.LinkNetworkFollower
import se.bofr.dspi.link.LinkNetworkMonitor
import se.bofr.dspi.link.LinkSettingsStore
import se.bofr.dspi.link.LinkTokenStore
import se.bofr.dspi.link.NsdHubBrowser
import se.bofr.dspi.link.PrefsKeyValue
import se.bofr.dspi.link.PrefsLinkSettingsStore
import se.bofr.dspi.link.UnsecuredAckStore
import se.bofr.dspi.link.keystoreLinkTokenStore
import se.bofr.dspi.ui.settings.AppSettingsStore
import se.bofr.dspi.ui.settings.DataStoreAppSettingsStore
import se.bofr.dspi.ui.state.AppState
import se.bofr.dspi.usb.AndroidBootloaderLocator
import se.bofr.dspi.usb.AndroidServiceControl
import se.bofr.dspi.usb.AndroidUsbSession
import se.bofr.dspi.usb.BootloaderLocator
import se.bofr.dspi.usb.ConnectionServiceLink
import se.bofr.dspi.usb.UiVisibilityTracker
import se.bofr.dspi.usb.UsbSession

/**
 * Process-wide wiring (decision D9, review W4). One real stack per process:
 * ```
 * FfiDspiCore (UniFFI, libdspi_ffi.so)
 *   -> DspiBridgeImpl(core, appScope, linkStore = DataStoreLinkStore, appVersion)
 *   -> AndroidUsbSession(this, bridge, appScope).start(this)
 * UiVisibilityTracker(appScope) { session.setUiVisible(it) (+ demo bridge) }
 * ```
 * The session starts (and attach intents are handed over) only after the
 * links file has been read ([UsbStartGate]), so the bridge's main-thread
 * `LinkStore.load` never waits for the disk.
 * The USB session is started here, not per Activity (UsbSession.start KDoc),
 * so the device stays open across Activity recreation while the foreground
 * service runs. Demo mode builds a separate stack on demand
 * ([AppModeController]).
 *
 * DSPi Link (REMOTE, opt-in, off by default): [linkSettings], the
 * Keystore-sealed [linkTokens] and [networking] (Settings › Networking);
 * the REMOTE stack is built on demand by [AppModeController.enterRemote]
 * (its own FfiDspiCore + DspiBridgeImpl + LinkSessionEngine). One
 * `connectedDevice` foreground service serves USB and REMOTE
 * ([SharedServiceControl]); its notification follows the bridge on screen.
 *
 * Also process-wide, provided to every screen by [MainActivity]: the one
 * preference store ([appSettings], decision F3; its analyser options are
 * pushed into the current bridge by [bindRtaOptions]), the shared selection
 * ([appState]).
 */
class DspiApplication : Application() {
    /** Main-thread scope for the bridge, the session and the mode controller; lives as long as the process. */
    val appScope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    lateinit var bridge: DspiBridge
        private set
    lateinit var session: UsbSession
        private set
    lateinit var modes: AppModeController
        private set

    /** Run [block] (anything that may open a DSPi) once the persistent links are loaded (review A3). */
    lateinit var usbGate: UsbStartGate
        private set

    /** The app's one preference store (macOS AppSettings, DataStore). */
    lateinit var appSettings: AppSettingsStore
        private set

    lateinit var linkSettings: LinkSettingsStore
        private set
    lateinit var linkTokens: LinkTokenStore
        private set
    lateinit var networking: AppNetworkingController

    /** -- WP SECAN -- the core's `link_parse_pair_link` (a `dspi://pair` link). */
    lateinit var parsePairLink: (String) -> LinkPairLink

    /** WP AN26: the About table, from the device stack's core in every mode. */
    lateinit var aboutJson: () -> String
        private set

    // -- WP AN19 --
    /** Getting Started and What's New, from the device stack's core in every mode. */
    lateinit var onboarding: OnboardingSource
        private set

    /** The main window's selection (macOS activeEqChannel / isOverviewMode). */
    val appState = AppState()

    /**
     * The one bootloader locator (macOS `BootloaderLocator`), shared by every
     * FirmwareUpdateView visit; the view starts it on open and stops it when
     * the visit ends. Created on first use.
     */
    val bootloaderLocator: BootloaderLocator by lazy { AndroidBootloaderLocator(this, appScope) }

    // -- WP P1AN2 -- the UI mode chosen per device (the Mode dialog)
    val uiModes: UiModeStore by lazy { PrefsUiModeStore(this) }

    /** -- WP P1AN2 -- the installer's limits profiles and PINs (`limits_set_dir`), app-private. */
    private fun limitsFolder(): String = java.io.File(filesDir, "limits").path

    // -- WP UAN -- the core's speaker layouts, one subfolder per serial (as the limits)
    private fun speakersFolder(): String = java.io.File(filesDir, "speakers").path

    // -- WP LKFAST -- the core's DSPi Link hub memory (a hub met without TLS, the hub bound last)
    private fun linkFolder(): String = java.io.File(filesDir, "dspi-link").path

    override fun onCreate() {
        super.onCreate()
        val version = appVersion()
        val linkStore = DataStoreLinkStore(dspiBridgeStore, appScope)
        val deviceCore = FfiDspiCore()
        runCatching { deviceCore.linkSetDir(linkFolder()) } // -- WP LKFAST -- a path only, no I/O
        aboutJson = deviceCore::aboutJson
        parsePairLink = deviceCore::linkParsePairLink // -- WP SECAN --
        onboarding = OnboardingSource(deviceCore::onboardingWhatsNewJson, deviceCore::onboardingGettingStartedJson) // -- WP AN19 --
        bridge = DspiBridgeImpl(deviceCore, appScope, linkStore = linkStore, appVersion = version)
        // -- WP P1AN2 -- named before a device opens (the FFI thread runs it before the USB attach)
        appScope.launch { bridge.installer.setFolder(limitsFolder()) }
        appScope.launch { bridge.speakers.setFolder(speakersFolder()) } // -- WP UAN -- (UXP B8) the speaker layouts
        val services = SharedServiceControl(AndroidServiceControl(this))
        val usb = AndroidUsbSession(this, bridge, appScope, service = services.client("usb"))
        session = usb
        linkSettings = PrefsLinkSettingsStore(this)
        linkTokens = keystoreLinkTokenStore(this)
        // -- WP AN4 -- before the REMOTE stack's factory, which marks a session active before its first dial
        lateinit var linkFollower: LinkNetworkFollower
        val network = LinkNetworkMonitor(this, onAvailable = { linkFollower.networkAvailable() }, onLost = { linkFollower.networkLost() })
        val clientName = { linkSettings.settings.value.clientName.ifBlank { Build.MODEL.orEmpty().ifBlank { "Android" } } }
        modes = AppModeController(
            deviceBridge = bridge,
            usbState = usb.state,
            usbDevices = usb.devices,
            scope = appScope,
            demoFactory = { startDemoSession(appScope, version, log = { Log.i(TAG, it) }) },
            // -- WP P1AN1 --
            documentFactory = { text, name -> startDocumentSession(appScope, version, text, name, log = { Log.i(TAG, it) }) },
            forwardUiVisible = usb::setUiVisible,
            remoteFactory = {
                network.setSessionActive(true)
                startRemoteSession(
                    appScope,
                    version,
                    FfiDspiCore().also { runCatching { it.linkSetDir(linkFolder()) } }, // -- WP LKFAST --
                    linkTokens,
                    clientName,
                    services.linkClient(),
                    linkStore,
                    log = { Log.i(TAG, it) },
                    onClose = { network.setSessionActive(false) },
                    findsHubs = { linkSettings.settings.value.findAutomatically } // -- WP LKFAST3 --
                ).also { r ->
                    appScope.launch {
                        r.bridge.installer.setFolder(limitsFolder()) // -- WP P1AN2 --
                        r.bridge.speakers.setFolder(speakersFolder()) // -- WP UAN --
                    }
                }
            },
            unsecuredAcks = UnsecuredAckStore(PrefsKeyValue(this, "dspi_link_unsecured_acks")), // -- WP ANSEC --
            // -- WP LKFAST -- the user left the hub: no dial at the next start (a small file write)
            leftHub = { appScope.launch(Dispatchers.IO) { runCatching { deviceCore.linkForgetLastHub() } } }
        )
        // The notification acts on the unit on screen (device, demo or REMOTE)
        ConnectionServiceLink.controls = SwitchingNotificationControls(modes.bridge, appScope)
        val hubBrowser = NsdHubBrowser(this, appScope, deviceCore::linkParseTxt)
        networking = AppNetworkingController(
            linkSettings,
            hubBrowser,
            linkTokens,
            modes,
            appScope,
            Build.MODEL.orEmpty(),
            parseAddress = { t -> runCatching { deviceCore.linkParseAddress(t) }.getOrNull()?.let { it.host to it.port } },
            credentialKey = deviceCore::linkCredentialHubKey, // -- WP SECAN --
            credentialTlsOnly = deviceCore::linkCredentialTlsOnly, // -- WP PINTLS --
            forgetHub = deviceCore::linkForgetHub, // -- WP LKFAST3 --
            rows = Dispatchers.Default,
            io = Dispatchers.IO,
            hubEncryption = deviceCore::linkHubEncryption // -- WP LKFAST -- a small file read per row
        )
        linkFollower = LinkNetworkFollower(hubBrowser, modes.remoteState) { modes.remoteSession?.networkTarget }
        appScope.launch { linkFollower.follow() }
        appScope.launch { linkSettings.settings.collect { if (it.enabled) network.start() else network.stop() } }
        usbGate = UsbStartGate(appScope) { linkStore.awaitLoaded() }
        usbGate.run {
            usb.start(this)
            // -- WP LKFAST -- (owner decision 2026-11-21) after the USB scan: USB wins
            val s = linkSettings.settings.value
            if (s.enabled && s.reconnectAtStart) {
                appScope.launch {
                    val scanned = suspend { withTimeoutOrNull(USB_SCAN_WAIT_MS) { usb.scanned.first { it } } }
                    modes.reconnectAtStart(usbScanned = { scanned() }) { usbPresent ->
                        withContext(Dispatchers.IO) { deviceCore.linkStartHub(usbPresent) }?.let(::lastHubOf)
                    }
                }
            }
        }
        UiVisibilityTracker(appScope) {
            modes.setUiVisible(it)
            if (it) linkFollower.uiShown()
        }.install(this)
        appSettings = DataStoreAppSettingsStore.of(this)
        bindRtaOptions(appScope, appSettings.settings, modes.bridge)
    }

    private fun appVersion(): String = runCatching {
        packageManager.getPackageInfo(packageName, 0).versionName.orEmpty()
    }.getOrDefault("")

    private companion object {
        const val TAG = "DspiApplication"

        // -- WP LKFAST3 -- the first USB scan is quick; never hold the start's dial longer
        const val USB_SCAN_WAIT_MS = 2_000L
    }
}
