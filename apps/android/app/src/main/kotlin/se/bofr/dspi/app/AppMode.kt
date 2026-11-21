package se.bofr.dspi.app

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DangerousOp
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.LinkLastHub
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.core.PresetStatus
import se.bofr.dspi.feature.presets.PresetMenuActions
import se.bofr.dspi.feature.presets.UnsavedChangesAction
import se.bofr.dspi.feature.presets.defaultPresetName
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.InMemoryKeyValue
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.UnsecuredAckStore
import se.bofr.dspi.link.linkBrowseWanted
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.usb.DspiUsbDevice
import se.bofr.dspi.usb.UsbSessionState

/** Which bridge the UI is looking at. */
enum class AppMode {
    /** The real stack: FfiDspiCore behind DspiBridgeImpl, fed by AndroidUsbSession. */
    DEVICE,

    /** "Demo (no device)": the real core's demo device behind its own DspiBridgeImpl; no USB at all. */
    DEMO,

    /** DSPi Link client: a DSPi on a hub (Mac Console, ESP32 bridge), its own core + bridge ([RemoteSession]). */
    REMOTE
}

/** A running demo stack; [close] tears it down completely (scope, FFI thread, core). */
interface DemoSession {
    val bridge: DspiBridge

    fun close()
}

/**
 * True while any DSPi is on the bus, opened or not (permission pending,
 * denied, failed...). Demo mode is offered only when this is false.
 */
fun dspiPresent(state: UsbSessionState, devices: List<DspiUsbDevice>): Boolean = state !is UsbSessionState.NoDevice || devices.isNotEmpty()

// -- WP LKFAST --
/** The core's last hub ([se.bofr.dspi.core.DspiCoreApi.linkStartHub]) as a browsed hub, to dial at start. */
fun lastHubOf(h: LinkLastHub): LinkHub =
    LinkHub(h.hubId, h.name, h.kind, h.auth, 1, emptyList(), h.host, h.port, h.path, tlsAdvertised = h.tlsAdvertised)

/**
 * Selects between the device bridge and an explicit, separate demo bridge
 * (PLAN §3.3 `AppState`). Rules:
 * - demo can be entered with or without a DSPi attached (WP AN7: one device list);
 * - the demo stack is its own core + bridge; the USB session only ever talks
 *   to [deviceBridge], so a real unit and the fake are never mixed;
 * - when a DSPi appears while in demo, demo is left at once and torn down,
 *   and the UI switches back to the device bridge;
 * - a switch the user asks for ([switchTo], the strip's buttons, the D9 prompt) runs the
 *   Unsaved Changes flow on the current bridge first (M42, [DeviceSwitchGate]);
 * - REMOTE (DSPi Link) is its own stack too; entering it leaves demo. When a
 *   DSPi appears on USB while REMOTE, the user is asked ([usbPrompt], owner
 *   decision D9) instead of switching silently. REMOTE ends when the user
 *   leaves the hub (the engine goes Idle) or chooses USB.
 *
 * Main thread only (as the bridge and the USB session).
 */
class AppModeController(
    private val deviceBridge: DspiBridge,
    private val usbState: StateFlow<UsbSessionState>,
    private val usbDevices: StateFlow<List<DspiUsbDevice>>,
    scope: CoroutineScope,
    private val demoFactory: () -> DemoSession,
    private val forwardUiVisible: (Boolean) -> Unit,
    private val remoteFactory: () -> RemoteSession = { error("DSPi Link is not available") },
    // -- WP P1AN1 -- a document opened offline: the demo stack on the document (P1OF)
    private val documentFactory: (text: String, fileName: String) -> DemoSession = { _, _ -> error("Opening a file is not available") },
    // -- WP ANSEC -- the hubs whose unsecured connection the user acknowledged
    val unsecuredAcks: UnsecuredAckStore = UnsecuredAckStore(InMemoryKeyValue()),
    // -- WP LKFAST -- the user left the hub: the core's `link_forget_last_hub` (no dial at the next start)
    private val leftHub: () -> Unit = {}
) {
    private val scope = scope
    private val _mode = MutableStateFlow(AppMode.DEVICE)
    val mode: StateFlow<AppMode> = _mode.asStateFlow()

    private val _bridge = MutableStateFlow(deviceBridge)

    /** The bridge every screen is given; changes on demo enter/exit. */
    val bridge: StateFlow<DspiBridge> = _bridge.asStateFlow()

    private var demo: DemoSession? = null
    private var uiVisible = true

    private val present: StateFlow<Boolean> =
        combine(usbState, usbDevices, ::dspiPresent)
            .stateIn(scope, SharingStarted.Eagerly, dspiPresent(usbState.value, usbDevices.value))

    private var remote: RemoteSession? = null
    private var remoteWatch: Job? = null
    private val _remoteState = MutableStateFlow<LinkSessionState>(LinkSessionState.Idle)

    /** The REMOTE session's state (Idle outside REMOTE). */
    val remoteState: StateFlow<LinkSessionState> = _remoteState.asStateFlow()

    // -- WP SECAN --
    private var securityWatch: Job? = null
    private val _remoteSecurity = MutableStateFlow<HubSecurity?>(null)

    /** The REMOTE session's encryption (null outside REMOTE): the red "(unsecured)" bar, the new-key notice. */
    val remoteSecurity: StateFlow<HubSecurity?> = _remoteSecurity.asStateFlow()

    // -- WP AN4 --
    private val uiShown = MutableStateFlow(true)

    /** The session needs DNS-SD now, whatever the Networking page does ([linkBrowseWanted]). */
    val browseForSession: StateFlow<Boolean> =
        combine(_remoteState, uiShown, ::linkBrowseWanted).stateIn(scope, SharingStarted.Eagerly, false)

    /** The REMOTE stack while it exists (dialogs act on it). */
    val remoteSession: RemoteSession? get() = remote

    private val _usbPrompt = MutableStateFlow(false)

    /** D9: a DSPi appeared on USB while REMOTE: ask whether to switch to it. */
    val usbPrompt: StateFlow<Boolean> = _usbPrompt.asStateFlow()

    // -- WP AN7 --
    /** The Unsaved Changes alert of a device switch (and "Save Failed"); the app shows it over every screen. */
    val switchAlerts = AlertHost()
    private val switchGate = DeviceSwitchGate({ _bridge.value }, scope, switchAlerts)

    init {
        scope.launch {
            var before = present.value
            present.collect {
                if (it) exitDemo()
                if (it && !before && _mode.value == AppMode.REMOTE) _usbPrompt.value = true
                if (!it) _usbPrompt.value = false
                before = it
            }
        }
    }

    // -- WP SECAN --
    /**
     * A `dspi://pair` link the user confirmed: switch to its hub (after the Unsaved Changes
     * flow, as any switch) and dial it pinned to the link's key.
     */
    fun pairByLink(link: LinkPairLink) {
        val hub = LinkHub(link.hubId, link.host, "unknown", "pin", 0, emptyList(), link.host, link.port)
        switchGate.run { enterRemote(hub, link) }
    }

    // -- WP LKFAST --
    /**
     * At start (owner decision 2026-11-21, B): [startHub] gives the hub used last, given
     * whether a DSPi is on USB (the core's `link_start_hub`: USB wins, none if the user
     * left it), and it is dialled at its stored address at once, as a browsed hub is: its
     * stored credential, its advert's TLS. Nothing if the user chose something already.
     */
    suspend fun reconnectAtStart(usbScanned: suspend () -> Unit = {}, startHub: suspend (usbPresent: Boolean) -> LinkHub?) {
        usbScanned() // -- WP LKFAST3 -- (review LKREV 8) "USB wins" on the first scan's list, not on luck
        if (_mode.value != AppMode.DEVICE || remote != null) return
        val hub = startHub(dspiPresent(usbState.value, usbDevices.value)) ?: return
        if (_mode.value != AppMode.DEVICE || remote != null || dspiPresent(usbState.value, usbDevices.value)) return
        enterRemote(hub, atStart = true)
    }

    /** Switch to the DSPi on [hub] (DSPi Link). Leaves demo; reuses the REMOTE stack when it exists. */
    fun enterRemote(hub: LinkHub, link: LinkPairLink? = null, atStart: Boolean = false) {
        exitDemo()
        val r = remote ?: remoteFactory().also { r ->
            remote = r
            r.bridge.setUiVisible(uiVisible)
            r.setUiVisible(uiVisible)
            remoteWatch = scope.launch {
                var active = false
                r.state.collect { st ->
                    _remoteState.value = st
                    if (st != LinkSessionState.Idle) {
                        active = true
                    } else if (active) {
                        exitRemote()
                    }
                }
            }
            // -- WP SECAN --, ANSEC: with the user's acknowledgement, dropped when the hub's security changed
            securityWatch = scope.launch {
                combine(r.security, r.state, unsecuredAcks.acks) { sec, st, _ -> acknowledgedOf(sec, st) }.collect {
                    _remoteSecurity.value =
                        it
                }
            }
        }
        when { // -- WP SECAN --, LKFAST
            link != null -> r.connectByLink(link)
            atStart -> r.connectAtStart(hub)
            else -> r.connect(hub)
        }
        _bridge.value = r.bridge
        _mode.value = AppMode.REMOTE
        _usbPrompt.value = false
    }

    // -- WP ANSEC --
    /** [sec] as the bar shows it: [HubSecurity.acknowledged] while the user's acknowledgement of its hub holds. */
    private fun acknowledgedOf(sec: HubSecurity?, st: LinkSessionState): HubSecurity? {
        val hub = st.hub
        if (sec == null || hub == null || !sec.hubId.equals(hub.id, ignoreCase = true) || !hubConnectionLive(st)) return sec
        unsecuredAcks.observe(hub, sec)
        return sec.copy(acknowledged = sec.unsecured && unsecuredAcks.acknowledged(hub))
    }

    /**
     * The user acknowledged that the current hub's connection is unsecured (the info dialog, the
     * strip's overflow): the bar is no longer highlighted for this hub. Only while it is unsecured.
     */
    fun acknowledgeUnsecured() {
        val hub = _remoteState.value.hub ?: return
        val sec = _remoteSecurity.value ?: return
        if (sec.unsecured && sec.hubId.equals(hub.id, ignoreCase = true)) unsecuredAcks.acknowledge(hub)
    }

    /** Leave REMOTE and tear its stack down; the UI goes back to the device bridge. No-op otherwise. */
    fun exitRemote() {
        val r = remote ?: return
        remote = null
        leftHub() // -- WP LKFAST -- left on purpose: no dial at the next start
        remoteWatch?.cancel()
        remoteWatch = null
        securityWatch?.cancel() // -- WP SECAN --
        securityWatch = null
        _remoteSecurity.value = null
        _remoteState.value = LinkSessionState.Idle
        _usbPrompt.value = false
        if (_mode.value == AppMode.REMOTE) {
            _bridge.value = deviceBridge
            _mode.value = AppMode.DEVICE
        }
        r.close()
    }

    /** The USB prompt's answer: true = use the USB DSPi (leave REMOTE, after the Unsaved Changes flow). */
    fun answerUsbPrompt(useUsb: Boolean) {
        _usbPrompt.value = false
        if (useUsb) switchTo(DeviceChoice.Usb)
    }

    // -- WP AN7 --
    /**
     * The user's device switch (the device list, "Exit demo", "Disconnect", Networking's
     * Connect): asks about unsaved changes on the current bridge first, then switches.
     * The current USB or demo choice and [DeviceChoice.Network] (a dialog, not a switch)
     * do nothing; the current hub again dials it again (Connect after a failure), with no prompt.
     */
    fun switchTo(choice: DeviceChoice) {
        val now = when (choice) {
            DeviceChoice.Usb -> _mode.value == AppMode.DEVICE
            DeviceChoice.Demo -> _mode.value == AppMode.DEMO
            is DeviceChoice.Hub -> _mode.value == AppMode.REMOTE && onHub(_remoteState.value, choice.hub)
            DeviceChoice.Network -> true
        }
        if (now) {
            if (choice is DeviceChoice.Hub) enterRemote(choice.hub)
            return
        }
        switchGate.run {
            when (choice) {
                DeviceChoice.Usb -> {
                    exitDemo()
                    exitRemote()
                }
                DeviceChoice.Demo -> {
                    exitRemote()
                    enterDemo()
                }
                is DeviceChoice.Hub -> enterRemote(choice.hub)
                DeviceChoice.Network -> Unit
            }
        }
    }

    /** Start demo mode, also with a DSPi attached (WP AN7). Refused (false) while REMOTE. Idempotent. */
    fun enterDemo(): Boolean {
        if (demo != null) return true
        if (remote != null) return false
        val d = demoFactory()
        demo = d
        d.bridge.setUiVisible(uiVisible)
        _bridge.value = d.bridge
        _mode.value = AppMode.DEMO
        return true
    }

    // -- WP P1AN1 --
    /**
     * Open File… (P1OF): leave the current device (after the Unsaved Changes
     * flow, as any switch) for a session on the document [text], named
     * [fileName]. It runs as a demo stack (mode DEMO, nothing on USB); the strip
     * says "File: <name>" while its bridge reports the document. Leaving it is
     * leaving demo. A document opened while one is open replaces it.
     */
    fun openDocument(text: String, fileName: String) {
        switchGate.run {
            exitRemote()
            exitDemo()
            val d = documentFactory(text, fileName)
            demo = d
            d.bridge.setUiVisible(uiVisible)
            _bridge.value = d.bridge
            _mode.value = AppMode.DEMO
        }
    }

    /** Leave demo mode and tear the demo stack down. No-op in device mode. */
    fun exitDemo() {
        val d = demo ?: return
        demo = null
        _bridge.value = deviceBridge
        _mode.value = AppMode.DEVICE
        d.close()
    }

    /** From UiVisibilityTracker: the USB session (and device bridge) plus the demo/remote bridge if any. */
    fun setUiVisible(visible: Boolean) {
        uiVisible = visible
        forwardUiVisible(visible)
        demo?.bridge?.setUiVisible(visible)
        remote?.bridge?.setUiVisible(visible)
        remote?.setUiVisible(visible)
        uiShown.value = visible
        if (visible) remote?.retryNow()
    }
}

// -- WP AN7 --
/**
 * M42 for a device switch: macOS `DSPViewModel.switchToDevice` asks with the
 * Unsaved Changes alert first (`PresetMenuActions.withUnsavedChanges`, the
 * diff from the core). Save writes the active slot (named "Preset N" if it has
 * no name) and switches only when that worked, else "Save Failed … Device
 * switch aborted."; Discard switches; Cancel stays. Discard reverts nothing
 * on the unit: it keeps its live state, as on macOS.
 */
class DeviceSwitchGate(private val bridge: () -> DspiBridge, private val scope: CoroutineScope, private val host: AlertHost) {
    fun run(switch: () -> Unit) {
        val b = bridge()
        PresetMenuActions(b, scope, host).withUnsavedChanges { action ->
            if (action != UnsavedChangesAction.SAVE) {
                switch()
                return@withUnsavedChanges
            }
            // Decision E1: the alert's Save handler mints the token
            val token = b.dangerousOps.confirm(DangerousOp.SAVE_PRESET)
            // E9: the save is not stopped halfway by a configuration change
            scope.launch(NonCancellable) {
                val slot = b.activePresetSlot
                if (b.presetName(slot).isEmpty()) b.setPresetName(slot, defaultPresetName(slot))
                val status = b.savePreset(token, slot)
                if (status == PresetStatus.OK) {
                    switch()
                } else {
                    host.showWarning("Save Failed", "Failed to save preset (${status.failureText}). Device switch aborted.")
                }
            }
        }
    }
}
