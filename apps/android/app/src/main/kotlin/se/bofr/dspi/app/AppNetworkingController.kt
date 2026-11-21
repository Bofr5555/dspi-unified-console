package se.bofr.dspi.app

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import se.bofr.dspi.link.HubBrowser
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.link.LinkSettingsStore
import se.bofr.dspi.link.LinkTokenStore
import se.bofr.dspi.link.ManualHub
import se.bofr.dspi.link.hubBadge
import se.bofr.dspi.ui.link.AcknowledgedHubRow
import se.bofr.dspi.ui.link.NetworkHubRow
import se.bofr.dspi.ui.link.NetworkingController
import se.bofr.dspi.ui.link.NetworkingUiState
import se.bofr.dspi.ui.link.PairedHubRow

/**
 * Settings › Networking's model ([NetworkingController]) over the Link
 * settings, the DNS-SD browser, the token store and the mode controller.
 * Main thread. [parseAddress] is the core's `link_parse_address`
 * (macOS `HubBrowser.parseAddress`). Browsing runs only while DSPi Link is
 * on, "Find hubs automatically" is on and the page is on screen, or the session
 * needs its hub found again ([AppModeController.browseForSession], WP AN4).
 * The connection line says what the strip says, in CG10's wording ([wording], WP AN5).
 */
class AppNetworkingController(
    private val settings: LinkSettingsStore,
    private val browser: HubBrowser,
    private val tokens: LinkTokenStore,
    private val modes: AppModeController,
    private val scope: CoroutineScope,
    private val defaultClientName: String,
    private val parseAddress: (String) -> Pair<String, Int>?,
    private val wording: LinkWording = LinkWording.CORE,
    // -- WP SECAN -- the core's `link_credential_hub_key`: the pin a stored credential carries ("" = none)
    private val credentialKey: (String) -> String = { "" },
    // -- WP PINTLS -- the core's `link_credential_tls_only`: the record keeps the hub TLS-only
    private val credentialTlsOnly: (String) -> Boolean = { false },
    // -- WP LKFAST -- the core's `link_recheck_tls`: a forgotten hub is met as new, TLS first
    private val recheckTls: (String) -> Unit = {}
) : NetworkingController {
    private val visible = MutableStateFlow(false)
    private val pairedRevision = MutableStateFlow(0)

    /** Every hub the page lists: browsed first, then the typed ones. */
    val hubs: StateFlow<List<LinkHub>> =
        combine(browser.hubs, settings.settings) { browsed, s ->
            browsed + s.manualHubs.map { LinkHub.manual(it.host, it.port) }
        }.stateIn(scope, SharingStarted.Eagerly, emptyList())

    override val state: StateFlow<NetworkingUiState> =
        combine(
            combine(settings.settings, hubs, browser.browsing, browser.error) { a, b, c, d -> Quad(a, b, c, d) },
            modes.remoteState,
            pairedRevision,
            modes.unsecuredAcks.acks // -- WP ANSEC --
        ) { (s, hubs, browsing, error), remote, _, acks ->
            NetworkingUiState(
                enabled = s.enabled,
                findAutomatically = s.findAutomatically,
                clientName = s.clientName,
                defaultClientName = defaultClientName,
                hubs = hubs.map { row(it, remote) },
                browsing = browsing,
                browseError = error,
                paired = tokens.paired().map { PairedHubRow(it.hubId, it.name, pinOf(it.hubId), tlsOnlyOf(it.hubId)) }, // -- WP SECAN --
                status = status(remote),
                connected = remote != LinkSessionState.Idle,
                // -- WP ANSEC --
                acknowledged = acks.map { a ->
                    val at = if (':' in a.host) "[${a.host}]:${a.port}" else "${a.host}:${a.port}"
                    AcknowledgedHubRow(a.hubId, a.name, if (a.typed) at else "")
                }
            )
        }.stateIn(scope, SharingStarted.Eagerly, NetworkingUiState())

    private data class Quad<A, B, C, D>(val a: A, val b: B, val c: C, val d: D)

    init {
        scope.launch {
            combine(settings.settings, visible, modes.browseForSession) { s, v, forSession ->
                s.enabled && s.findAutomatically && (v || forSession)
            }.distinctUntilChanged().collect { on ->
                if (on) browser.start() else browser.stop()
            }
        }
        scope.launch {
            // A pairing just stored (or dropped) shows up in Paired Hubs
            modes.remoteState.collect { pairedRevision.value++ }
        }
    }

    private fun row(h: LinkHub, remote: LinkSessionState): NetworkHubRow {
        val n = h.deviceCount
        val detail = if (h.isManual) h.displayAddress else "${h.displayAddress}, $n device${if (n == 1) "" else "s"}"
        val active = remote.hub?.let { it.id == h.id || (h.isManual && it.host == h.host && it.port == h.port) } == true
        val badge = hubBadge(h, pinOf(h.id).isNotEmpty()).label // -- WP SECAN --
        return NetworkHubRow(h.id, h.name, detail, h.kind, h.isOpen, h.isManual, active && remote != LinkSessionState.Idle, badge)
    }

    // -- WP SECAN --
    /** The key fingerprint the stored credential of [hubId] is pinned to, "" when none. */
    private fun pinOf(hubId: String): String = tokens.token(hubId)?.let { runCatching { credentialKey(it) }.getOrNull() }.orEmpty()

    // -- WP PINTLS --
    private fun tlsOnlyOf(hubId: String): Boolean = tokens.token(hubId)?.let { runCatching { credentialTlsOnly(it) }.getOrNull() } == true

    private fun status(s: LinkSessionState): String = when (s) {
        LinkSessionState.Idle -> ""
        is LinkSessionState.Connected -> "${s.device.name.ifEmpty { "DSPi" }} via ${s.hub.name}"
        is LinkSessionState.Connecting -> "Connecting to ${s.hub.name}…"
        // -- WP AN5 -- every other state as the strip says it, so the two never disagree
        else -> remoteStrip(s, wording = wording).let { if (it.detail.isEmpty()) it.text else "${it.text} · ${it.detail}" }
    }

    override fun setEnabled(enabled: Boolean) {
        settings.update { it.copy(enabled = enabled) }
        if (!enabled) modes.exitRemote()
    }

    override fun setFindAutomatically(on: Boolean) = settings.update { it.copy(findAutomatically = on) }

    override fun setClientName(name: String) = settings.update { it.copy(clientName = name) }

    override fun connect(hubId: String) {
        if (!settings.settings.value.enabled) return
        // -- WP AN7 -- a user's switch: the Unsaved Changes flow first
        hubs.value.firstOrNull { it.id == hubId }?.let { modes.switchTo(DeviceChoice.Hub(it)) }
    }

    override fun disconnect() = modes.switchTo(DeviceChoice.Usb) // -- WP AN7 --

    override fun isValidAddress(text: String): Boolean = parseAddress(text) != null

    override fun addManual(text: String): Boolean {
        val (host, port) = parseAddress(text) ?: return false
        settings.update { s -> s.copy(manualHubs = s.manualHubs.filterNot { it.host == host && it.port == port } + ManualHub(host, port)) }
        return true
    }

    override fun removeManual(hubId: String) {
        settings.update { s -> s.copy(manualHubs = s.manualHubs.filterNot { LinkHub.manualId(it.host, it.port) == hubId }) }
    }

    override fun forget(hubId: String) {
        tokens.remove(hubId)
        recheckTls(hubId) // -- WP LKFAST --
        pairedRevision.value++
    }

    override fun setVisible(visible: Boolean) {
        this.visible.value = visible
    }

    // -- WP ANSEC --
    override fun acknowledgeUnsecured() = modes.acknowledgeUnsecured()

    override fun highlightUnsecuredAgain() {
        modes.remoteState.value.hub?.let { modes.unsecuredAcks.forget(it.id) }
    }

    override fun forgetAcknowledgement(hubId: String) = modes.unsecuredAcks.forget(hubId)

    override fun forgetAcknowledgements() = modes.unsecuredAcks.forgetAll()
}
