// App-side policy around the core's Link session (WP AN3). The core keeps the
// session up by itself (WP C15): it reconnects with backoff, re-attaches the
// bound board and refreshes the mirror. This engine dials once, follows
// `linkStatus().phase` and adds what only an app knows: the stored tokens, the
// board the user wants, the PIN prompt, the foreground service.
//
// It has no retry loop and queues no write. The core's first dial retries by itself
// for up to 60 s (timeouts, a lost socket, a busy hub; never a refusal: WP LK4), all
// of it shown as Connecting, with Disconnect (`linkClose`) as the way out. One that
// still fails has no session and so no backoff in the core: the app repeats it once
// for each thing that happened (the user's tap, a network coming up, the hub
// announced again; WP AN4), never on a timer, and never a second dial on top of one
// still trying (the dial lock; an edge during it re-dials after it). So a hub that
// stays away costs nothing after that minute.
//
// Ported from DSPi-Console release/v1.1.6 `Link/NetworkTransport.swift`, whose
// differences stay: the pairing prompt is a non-modal state the UI shows
// ([LinkSessionState.NeedsPin]), and a list of several boards asks the user
// ([LinkSessionState.ChooseDevice]) instead of macOS's device menu.
package se.bofr.dspi.link

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkEncryption
import se.bofr.dspi.core.LinkHello
import se.bofr.dspi.core.LinkListener
import se.bofr.dspi.core.LinkPairLink
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.LinkStatus

/** Where the REMOTE connection stands. */
sealed interface LinkSessionState {
    val hub: LinkHub?

    data object Idle : LinkSessionState {
        override val hub: LinkHub? = null
    }

    data class Connecting(override val hub: LinkHub) : LinkSessionState

    /**
     * The hub wants a PIN: show the pairing prompt. [pinSource] `window` or `fixed` (spec 6.2).
     * [errorCode]: the `DspiException.Link` code of the last failed pair (`wrong_pin`,
     * `rate_limited`, ...), empty when [error] is not a pair's answer (WP AN36).
     * -- WP SECAN -- [hubKey]: the fingerprint of the key the hub presented over TLS, to
     * compare with the one the hub shows ("" = unencrypted: the PIN crosses in clear).
     * [pin]: the PIN a pairing link carried, filled in; the user still taps Pair.
     */
    data class NeedsPin(
        override val hub: LinkHub,
        val pinSource: String,
        val error: String = "",
        val busy: Boolean = false,
        val errorCode: String = "",
        val hubKey: String = "",
        val pin: String = "",
        // -- WP LKFAST -- the session's encryption as the prompt words it (the core's rule)
        val encryption: LinkEncryption = LinkEncryption.UNKNOWN
    ) : LinkSessionState

    /** The hub shares several boards and none was chosen yet. */
    data class ChooseDevice(override val hub: LinkHub, val devices: List<LinkDevice>) : LinkSessionState

    /** Connected to the hub, but [serial] (or any board, when empty) is not online there. */
    data class WaitingForDevice(override val hub: LinkHub, val serial: String) : LinkSessionState

    /** Controlling [device] through [hub]; [role] `viewer`, `control` or `admin`. */
    data class Connected(override val hub: LinkHub, val device: LinkDevice, val role: String) : LinkSessionState

    /**
     * The core lost the hub or the board and is reconnecting; the next try is at
     * [retryAtMs]. The mirror stays readable and writes are refused. [text] is the
     * core's wording for it (`link_phase_text`), [reason] the core's `close_reason`.
     * [parked]: out of sight for so long that the foreground service was let go.
     */
    data class Reconnecting(
        override val hub: LinkHub,
        val reason: String,
        val retryAtMs: Long,
        val attempt: Int,
        val text: String,
        val parked: Boolean = false
    ) : LinkSessionState

    /**
     * Ended and not retried on a timer (no first connection, a protocol error, refused): see [LinkSessionEngine.networkAvailable].
     * -- WP SECAN -- [code]: the failed dial's `DspiException.Link` code ("" otherwise); one of
     * [REPAIR_CODES] offers pairing again ([LinkSessionEngine.rePair]). [fromPairLink]: the dial
     * was a pairing link's, whose key the hub did not have.
     */
    data class Failed(override val hub: LinkHub, val reason: String, val code: String = "", val fromPairLink: Boolean = false) :
        LinkSessionState
}

/** The foreground service while a remote device is open (`connectedDevice`). */
interface LinkServiceControl {
    fun start(): Boolean

    fun stop()

    companion object {
        val NONE = object : LinkServiceControl {
            override fun start() = true

            override fun stop() {}
        }
    }
}

/**
 * One REMOTE stack's policy. Main thread only (all calls and state).
 * [clientName] is read at every connect (Settings may change it).
 *
 * The listener goes on the core once, here, and stays for the core's life: a
 * listener's callback must not call `setLinkListener`, and nothing else needs to.
 */
class LinkSessionEngine(
    private val ops: LinkCoreOps,
    private val tokens: LinkTokenStore,
    private val clientName: () -> String,
    private val appVersion: String,
    private val scope: CoroutineScope,
    private val service: LinkServiceControl = LinkServiceControl.NONE,
    private val clock: () -> Long = System::currentTimeMillis,
    private val log: (String) -> Unit = {}
) : LinkNetworkTarget {
    private val _state = MutableStateFlow<LinkSessionState>(LinkSessionState.Idle)
    val state: StateFlow<LinkSessionState> = _state.asStateFlow()

    /** The hub the user wants (null = none: Idle). */
    private var hub: LinkHub? = null

    /** The board to bind to, by serial (kept across reconnects, macOS `bind(serial:)`). */
    private var wantedSerial: String? = null
    private var attached: LinkDevice? = null
    private var role = "admin"

    /** The bridge was told the session is interrupted ([LinkCoreOps.setInterrupted]). */
    private var interrupted = false
    private var serviceOn = false

    /** A session of ours exists. Not during a dial, when the core's status still describes the one before it. */
    private var sessionLive = false

    /** This refusal's token is gone already; a late status must not delete the one a pairing stored since. */
    private var refusalHandled = false

    /** Bumped for every connect and teardown: whatever an older one was doing stops. */
    private var generation = 0
    private var teardown: Job? = null

    // -- WP AN4 --

    /** Something changed while a first dial was running: if that dial fails, one more. */
    private var redialAfterDial = false
    private var uiVisible = true
    private var parked = false
    private var parkTimer: Job? = null

    /** One dial at a time: `linkConnect` replaces any session, so the last to finish would win. */
    private val dialLock = Mutex()

    /** "Look at the core's status again". Conflated: one look covers any number of events. */
    private val wake = Channel<Unit>(Channel.CONFLATED)

    val attachedSerial: String? get() = attached?.serial

    // -- WP SECAN --
    private val _security = MutableStateFlow<HubSecurity?>(null)

    /** The session's encryption, null with no session (or before its first look). */
    val security: StateFlow<HubSecurity?> = _security.asStateFlow()

    /** "hubId|key" whose new-key notice was shown (this process; the core says it once per pin anyway). */
    private val keysNoticed = mutableSetOf<String>()

    /** The pairing link this dial is for: its pinned credential goes out instead of the stored one. */
    private var pairLink: LinkPairLink? = null

    // -- WP LKFAST --
    /** This dial is the app start's, to the hub used last, at its stored address ([connectAtStart]). */
    private var atStart = false

    // -- WP LKFAST3 --
    /** What the next dial does first, on its own coroutine (the core's hub memory: Pair Again, Forget). */
    private var beforeDial: (suspend () -> Unit)? = null

    /** Settings › Networking "Find hubs automatically" is on: a hub not at its address is looked for. */
    var findsHubs: () -> Boolean = { true }

    init {
        ops.setListener(
            object : LinkListener {
                override fun onLinkDevicesChanged() {
                    wake.trySend(Unit)
                }

                override fun onLinkClosed(code: Int, reason: String) {
                    wake.trySend(Unit)
                }

                override fun onLinkStatusChanged() {
                    wake.trySend(Unit)
                }

                override fun onLinkTokenIssued(hubId: String, token: String) {
                    scope.launch { storeToken(hubId, token) }
                }
            }
        )
        // Follows one event at a time, so an attach never overlaps the next look
        scope.launch { for (w in wake) follow() }
    }

    /** Connect to [target] (a user tap), optionally to board [serial]. */
    fun connect(target: LinkHub, serial: String? = null) = connect(target, serial, null)

    // -- WP LKFAST --
    /**
     * Connect to the hub used last when the app starts (owner decision 2026-11-21), at its
     * stored address, without waiting for DNS-SD. The same rules as a tap: its stored
     * credential (a pinned hub stays TLS only), the advert it had. If the address does not
     * answer, the hub is looked for on the network quietly: a short status line, and one
     * dial where it is seen ([hubSeen]), at once if it is seen elsewhere while this dial
     * is still trying.
     */
    fun connectAtStart(target: LinkHub) = connect(target, null, null, atStart = true)

    // -- WP SECAN --
    /**
     * A `dspi://pair` link the user confirmed ([LinkPairLink], the core's parse). The dial
     * uses its credential, pinned to its `k`: a hub with another key fails it with
     * `hub_key_unverified` before any PIN goes out. The prompt then shows the hub's name and
     * key with the link's PIN filled in, and pairs only on the user's tap.
     */
    fun connectByLink(link: LinkPairLink) {
        val target = LinkHub(link.hubId, link.host, "unknown", "pin", 0, emptyList(), link.host, link.port)
        connect(target, null, link)
    }

    /**
     * Pair again after one of [REPAIR_CODES]: the stored token (and its pin) is dropped and
     * the hub dialled, so it asks for a PIN and shows its key. After a pairing link the hub
     * did not match, the stored credential stays: the link was wrong, not the pairing.
     * -- WP PINTLS -- A hub that was pinned keeps the core's unpaired record, so it pairs over
     * TLS only ([dropPairing]); after [FORGET_HUB_CODE] this is Forget Hub: the record goes,
     * the only way back to an unencrypted pairing (owner decision 2026-11-17).
     */
    fun rePair() {
        val s = _state.value as? LinkSessionState.Failed ?: return
        if (s.code !in REPAIR_CODES) return
        val id = s.hub.id
        if (s.code == FORGET_HUB_CODE) {
            // -- WP LKFAST3 -- (review LKREV 2) forgotten in the core too, before the dial below
            beforeDial = { ops.forgetHub(id) }
            tokens.remove(s.hub.id)
            log("link: forgetting the hub (${s.code})")
        } else {
            beforeDial = { ops.recheckTls(id) } // -- WP LKFAST -- Pair Again tries TLS first again
            if (!s.fromPairLink) dropPairing(s.hub.id, s.hub.name)
            log("link: pairing again (${s.code})")
        }
        connect(s.hub, wantedSerial, null)
    }

    // -- WP PINTLS --
    /** The token goes; a hub that was ever pinned keeps a record that stays TLS-only. */
    private fun dropPairing(hubId: String, name: String) {
        val kept = ops.credentialUnpaired(tokens.token(hubId).orEmpty())
        if (kept.isEmpty()) {
            tokens.remove(hubId)
        } else {
            val known = tokens.paired().firstOrNull { it.hubId.equals(hubId, ignoreCase = true) }?.name
            tokens.setToken(hubId, known ?: name, kept)
        }
    }

    /** The new-key notice was seen. */
    fun ackKeyNotice() {
        val s = _security.value ?: return
        keysNoticed += s.hubId.lowercase() + "|" + s.hubKey
        _security.value = s.copy(keyNotice = false)
    }

    private fun connect(target: LinkHub, serial: String?, link: LinkPairLink?, atStart: Boolean = false) {
        pairLink = link
        this.atStart = atStart // -- WP LKFAST --
        _security.value = null
        val sameHub = hub?.id == target.id
        hub = target
        wantedSerial = serial ?: wantedSerial.takeIf { sameHub }
        refusalHandled = false
        val gen = ++generation
        sessionLive = false
        redialAfterDial = false // this dial starts after whatever happened
        unpark()
        _state.value = LinkSessionState.Connecting(target)
        scope.launch { dial(target, gen) }
    }

    /** The user typed a PIN in the prompt. `linkPair` dials again if the hub has dropped the socket meanwhile. */
    fun submitPin(pin: String) {
        val s = _state.value as? LinkSessionState.NeedsPin ?: return
        _state.value = s.copy(busy = true, error = "", errorCode = "")
        val gen = generation
        scope.launch {
            val token = try {
                ops.pair(pin)
            } catch (e: DspiException) {
                val code = (e as? DspiException.Link)?.code.orEmpty()
                if (gen == generation) _state.value = s.copy(busy = false, error = pairingError(e), errorCode = code)
                return@launch
            }
            if (gen != generation) return@launch
            if (token.isNotEmpty()) tokens.setToken(s.hub.id, s.hub.name, token)
            pairLink = null // -- WP SECAN -- paired: the stored credential from here on
            wake.trySend(Unit)
        }
    }

    /** The user cancelled the prompt: give the connection up (macOS). */
    fun cancelPin() {
        if (_state.value is LinkSessionState.NeedsPin) disconnect()
    }

    fun chooseDevice(device: LinkDevice) {
        val s = _state.value as? LinkSessionState.ChooseDevice ?: return
        wantedSerial = device.serial
        _state.value = LinkSessionState.Connecting(s.hub)
        wake.trySend(Unit)
    }

    /** Try now instead of waiting out the core's backoff (app back in front, network back, the Retry button). */
    fun retryNow() {
        if (_state.value is LinkSessionState.Reconnecting) scope.launch { ops.retryNow() }
    }

    // -- WP AN4 --

    /**
     * A network came up: the core's backoff is cut short, and a session that has
     * ended gets one more dial. One per call; there is no timer behind it.
     */
    override fun networkAvailable() {
        when (_state.value) {
            is LinkSessionState.Reconnecting -> retryNow()
            is LinkSessionState.Failed -> redial()
            is LinkSessionState.Connecting -> if (!sessionLive) redialAfterDial = true
            else -> Unit
        }
    }

    /** A network went: what is left may reach the hub. An ended session waits for one to come up. */
    override fun networkLost() = retryNow()

    /**
     * DNS-SD announces the wanted hub ([found] has its id). At another address the
     * core goes there (`link_set_url`, which the hub's id still guards); an ended
     * session gets one more dial, there. Writes are not involved: they failed already.
     */
    override fun hubSeen(found: LinkHub) {
        val wanted = hub ?: return
        val shown = _state.value
        if (!found.id.equals(wanted.id, ignoreCase = true) && !found.id.equals(shown.hub?.id, ignoreCase = true)) return
        val moved = found.host != wanted.host || found.port != wanted.port || found.path != wanted.path
        // -- WP LKFAST3 -- (review LKREV 4) the live advert's TLS: a sighting with tls=1 makes the redial TLS only
        val tls = wanted.tlsAdvertised || found.tlsAdvertised
        val there = (if (moved) wanted.copy(host = found.host, port = found.port, path = found.path) else wanted)
            .copy(tlsAdvertised = tls)
        when (shown) {
            is LinkSessionState.Reconnecting -> {
                if (moved) {
                    hub = there
                    _state.value = shown.copy(hub = shown.hub.copy(host = there.host, port = there.port, path = there.path))
                    log("link: the hub moved")
                    scope.launch { ops.setUrl(ops.wsUrl(there.host, there.port, there.path)) }
                } else {
                    retryNow()
                }
            }
            is LinkSessionState.Failed -> {
                hub = there
                redial()
            }
            // The dial under way may still work. If it does not, one more at the new address.
            is LinkSessionState.Connecting -> if (moved && !sessionLive) {
                hub = there
                if (atStart) {
                    // -- WP LKFAST -- the stored address is old: stop that dial (it may try for a
                    // minute) and dial where the hub is now
                    log("link: the hub used last is at another address now")
                    val gen = generation
                    scope.launch {
                        ops.close()
                        if (gen == generation) connect(there, wantedSerial, null, atStart = true)
                    }
                } else {
                    redialAfterDial = true
                }
            }
            else -> Unit
        }
    }

    /**
     * The UI went away or came back. Reconnecting for [RECONNECT_SERVICE_BOUND_MS]
     * out of sight lets the service go; the session stays and the core keeps trying
     * for as long as the system lets the process run. Coming back takes it up again.
     */
    fun setUiVisible(visible: Boolean) {
        uiVisible = visible
        if (!visible) {
            armPark()
            return
        }
        val wasParked = parked
        unpark()
        if (wasParked && attached != null) serviceRunning(true)
        // The bound state is looked at again, as a service start refused in the background may now work
        wake.trySend(Unit)
    }

    private fun redial() {
        hub?.let { connect(it, wantedSerial) }
    }

    private fun armPark() {
        if (uiVisible || parked || parkTimer?.isActive == true || _state.value !is LinkSessionState.Reconnecting) return
        parkTimer = scope.launch {
            delay(RECONNECT_SERVICE_BOUND_MS)
            val s = _state.value as? LinkSessionState.Reconnecting ?: return@launch
            parked = true
            serviceRunning(false)
            _state.value = s.copy(parked = true)
            log("link: reconnecting for long out of sight, the service stops")
        }
    }

    private fun unpark() {
        parkTimer?.cancel()
        parkTimer = null
        if (!parked) return
        parked = false
        (_state.value as? LinkSessionState.Reconnecting)?.let { _state.value = it.copy(parked = false) }
    }

    /** Leave REMOTE: end the session, reconnecting included, and forget the hub. Idempotent. */
    fun disconnect() {
        val had = hub != null || attached != null
        forgetSession()
        if (had) teardown = scope.launch { closeSession() }
    }

    /** [disconnect], then wait for the session to be closed and the device detached (stack teardown). */
    suspend fun close() {
        forgetSession()
        teardown?.join()
        closeSession()
    }

    /** Forget the token stored for [hubId] (Settings › Networking). */
    fun forget(hubId: String) {
        tokens.remove(hubId)
        scope.launch { ops.forgetHub(hubId) } // -- WP LKFAST3 -- met as new: TLS first, no start dial
    }

    private fun forgetSession() {
        generation++
        sessionLive = false
        redialAfterDial = false
        unpark()
        hub = null
        wantedSerial = null
        pairLink = null // -- WP SECAN --
        _security.value = null
        _state.value = LinkSessionState.Idle
    }

    /** `linkClose` first: it ends a command stuck on a stalled link, which the detach would queue behind. */
    private suspend fun closeSession() {
        ops.close()
        detachCurrent()
    }

    // ── The dial ──────────────────────────────────────────────────────

    private suspend fun dial(target: LinkHub, gen: Int): Unit = dialLock.withLock {
        // A `linkClose` still on its way would end the session this dial opens
        teardown?.join()
        beforeDial?.let { b ->
            beforeDial = null
            b()
        }
        if (gen != generation) return@withLock
        detachCurrent()
        var h = target
        if (h.isManual && h.id.startsWith(MANUAL_ID_PREFIX)) {
            // A typed address: its info document names the hub (for the token) and its path
            runCatching { ops.fetchInfo(h.host, h.port) }.getOrNull()?.let { i ->
                h =
                    h.copy(
                        id = i.hubId.ifEmpty {
                            h.id
                        },
                        name = i.hubName.ifEmpty { h.name },
                        kind = i.hubKind,
                        auth = i.auth,
                        path = i.wsPath
                    )
            }
            if (gen != generation) return@withLock
        }
        val known = !h.id.startsWith(MANUAL_ID_PREFIX)
        val first = open(h, gen, if (known) h.id else "") ?: return@withLock
        val named = merge(h, first)
        // Only the hello named this hub: dial once more, now that its token can go to it alone
        // -- WP PINTLS -- (review F3) also when the answer asks for no PIN and the record keeps the
        // hub TLS-only (pinned, or pinned before): no unencrypted or unchecked session in its name
        val record = if (known) null else tokens.token(named.id)
        val second = if (record != null && (first.needsPairing || ops.credentialUnpaired(record).isNotEmpty())) {
            open(named, gen, named.id) ?: return@withLock
        } else {
            first
        }
        val final = merge(named, second)
        hub = final
        log("link: connected to ${final.name} (${final.kind}, auth ${second.auth}, 1.${second.protoMinor})")
        if (second.tokenRefused) refuseToken(final)
        sessionLive = true
        readSecurity(final, gen) // -- WP SECAN --
        wake.trySend(Unit)
    }

    /** `linkConnect`, or null when it failed or was overtaken. Never leaves a session behind it for nobody. */
    private suspend fun open(h: LinkHub, gen: Int, expectedHubId: String): LinkHello? {
        // -- WP SECAN -- a pairing link's dial: its pinned credential, never the stored one
        val link = pairLink?.takeIf { expectedHubId.isNotEmpty() && it.hubId.equals(expectedHubId, ignoreCase = true) }
        val token = when {
            link != null -> link.credential
            expectedHubId.isEmpty() -> ""
            else -> tokens.token(expectedHubId).orEmpty()
        }
        val hello = try {
            val url = ops.wsUrl(h.host, h.port, h.path)
            if (atStart) { // -- WP LKFAST3 -- (review LKREV 6) a short first dial, then the search
                ops.connectAtStart(url, clientName(), appVersion, token, expectedHubId, h.tlsAdvertised)
            } else {
                ops.connect(url, clientName(), appVersion, token, expectedHubId, h.tlsAdvertised)
            }
        } catch (e: DspiException) {
            if (gen == generation) {
                val code = (e as? DspiException.Link)?.code.orEmpty()
                // -- WP LKFAST -- the start's dial: quietly, it is looked for on the network.
                // -- WP LKFAST3 -- (review LKREV 5) only when nothing answered there, and
                // "looking for it" only while DNS-SD looks
                val text = when {
                    !atStart || code !in NOT_THERE_CODES -> describe(e)
                    findsHubs() -> AT_START_NOT_FOUND
                    else -> AT_START_NOT_FOUND_NO_SEARCH
                }
                _state.value = LinkSessionState.Failed(h, text, code, fromPairLink = link != null)
                log("link: could not connect (${(e as? DspiException.Link)?.code ?: "error"})")
                if (redialAfterDial) redial()
            }
            return null
        }
        if (gen != generation) {
            ops.close()
            return null
        }
        return hello
    }

    private fun merge(h: LinkHub, hello: LinkHello) = h.copy(
        id = hello.hubId.ifEmpty { h.id },
        name = hello.hubName.ifEmpty { h.name },
        kind = hello.hubKind.ifEmpty { h.kind },
        auth = hello.auth.ifEmpty { h.auth }
    )

    // ── Following the core ────────────────────────────────────────────

    private suspend fun follow() {
        if (!sessionLive || hub == null) return
        val gen = generation
        val st = ops.status()
        val devices = ops.devices()
        if (gen != generation) return
        val h = hub ?: return
        readSecurity(h, gen) // -- WP SECAN --
        if (gen != generation) return
        st.role.takeIf { it.isNotEmpty() }?.let { role = it }
        if (st.phase != LinkPhase.RECONNECTING) unpark()
        when (st.phase) {
            LinkPhase.BOUND -> bound(h, devices, gen)
            LinkPhase.CHOOSE_DEVICE -> choose(h, devices, gen)
            LinkPhase.NO_DEVICE -> {
                markInterrupted()
                _state.value = LinkSessionState.WaitingForDevice(h, wantedSerial.orEmpty())
            }
            LinkPhase.RECONNECTING -> {
                markInterrupted()
                _state.value = LinkSessionState.Reconnecting(
                    h,
                    st.closeReason,
                    clock() + st.reconnectInMs,
                    st.reconnectAttempt.toInt(),
                    ops.phaseText(st.phase, st.pinSource).text,
                    parked
                )
                armPark()
            }
            LinkPhase.NEEDS_PAIRING -> needsPin(h, st)
            LinkPhase.CLOSED, LinkPhase.REFUSED, LinkPhase.IDLE -> ended(h, st)
        }
    }

    private suspend fun bound(h: LinkHub, devices: List<LinkDevice>, gen: Int) {
        val current = attached ?: return
        val d = devices.firstOrNull { it.serial == current.serial } ?: current
        attached = d
        ops.setReadOnly(role == VIEWER)
        if (interrupted) {
            // The core has re-attached and refreshed the mirror; its analyser worker ended with the old connection.
            if (!ops.deviceOpen()) {
                // A status poll saw the connection between the core's close and open, and the bridge let go of it
                attach(h, d, gen)
                return
            }
            ops.restartAnalyser()
            ops.refreshLock() // -- WP MERGEP1D -- the hub's lock, again after the reconnect
            if (gen != generation) return
            ops.setInterrupted(false)
            interrupted = false
        }
        serviceRunning(true)
        _state.value = LinkSessionState.Connected(h, d, role)
    }

    private suspend fun choose(h: LinkHub, devices: List<LinkDevice>, gen: Int) {
        val online = devices.filter { it.state == "online" || it.state.isEmpty() }
        val want = wantedSerial
        val pick = when {
            want != null -> online.firstOrNull { it.serial == want }
            online.size == 1 -> online[0]
            else -> null
        }
        when {
            pick != null -> attach(h, pick, gen)
            want != null -> _state.value = LinkSessionState.WaitingForDevice(h, want)
            online.isEmpty() -> _state.value = LinkSessionState.WaitingForDevice(h, "")
            else -> _state.value = LinkSessionState.ChooseDevice(h, online)
        }
    }

    private suspend fun attach(h: LinkHub, d: LinkDevice, gen: Int) {
        _state.value = LinkSessionState.Connecting(h)
        wantedSerial = d.serial
        ops.setReadOnly(role == VIEWER)
        val ok = ops.attach(d.handle, d.serial)
        if (gen != generation) {
            if (ok) ops.detach(d.serial)
            return
        }
        if (!ok) {
            _state.value = LinkSessionState.Failed(h, "Could not open the DSPi on ${h.name}.")
            return
        }
        attached = d
        ops.refreshLock() // -- WP MERGEP1D -- the hub's lock of the device just bound
        ops.setInterrupted(false)
        interrupted = false
        wake.trySend(Unit)
    }

    private fun needsPin(h: LinkHub, st: LinkStatus) {
        if (st.tokenRefused && !refusalHandled) refuseToken(h)
        if (!st.tokenRefused) refusalHandled = false
        // Nobody reconnects without the user now, so no service for it. The core re-attaches after the pairing.
        serviceRunning(false)
        markInterrupted()
        val shown = _state.value as? LinkSessionState.NeedsPin
        val source = st.pinSource.ifEmpty { "window" }
        // -- WP SECAN -- the key the hub presented, and a pairing link's PIN
        val key = _security.value?.takeIf { it.encrypted && it.hubId.equals(h.id, ignoreCase = true) }?.hubKey.orEmpty()
        val linkPin = pairLink?.takeIf { it.hubId.equals(h.id, ignoreCase = true) }?.pin.orEmpty()
        val enc = encryptionOf(_security.value?.takeIf { it.hubId.equals(h.id, ignoreCase = true) }) // -- WP LKFAST --
        _state.value = when {
            shown != null && shown.hub.id == h.id -> shown.copy(hub = h, pinSource = source, hubKey = key, encryption = enc)
            st.closeCode == CLOSE_REVOKED -> LinkSessionState.NeedsPin(
                h,
                source,
                error = "Access to ${h.name} was revoked. Pair again to reconnect.",
                hubKey = key,
                encryption = enc
            )
            else -> LinkSessionState.NeedsPin(h, source, hubKey = key, pin = linkPin, encryption = enc)
        }
    }

    private suspend fun ended(h: LinkHub, st: LinkStatus) {
        detachCurrent()
        val text = ops.phaseText(st.phase, st.pinSource).text
        _state.value = LinkSessionState.Failed(
            h,
            when {
                st.closeCode == CLOSE_PROTOCOL_ERROR -> "The hub reported a protocol error."
                st.closeReason.isNotEmpty() -> "$text: ${st.closeReason}"
                else -> text
            }
        )
    }

    /** The one place a stored token goes away by itself: the hub said it is bad. */
    private fun refuseToken(h: LinkHub) {
        refusalHandled = true
        dropPairing(h.id, h.name) // -- WP PINTLS -- a hub once pinned stays TLS-only
        log("link: the hub refused the stored token")
    }

    private fun storeToken(hubId: String, token: String) {
        if (token.isEmpty()) return
        val name = hub?.takeIf { it.id.equals(hubId, ignoreCase = true) }?.name
            ?: tokens.paired().firstOrNull { it.hubId.equals(hubId, ignoreCase = true) }?.name
            ?: hubId
        tokens.setToken(hubId, name, token)
    }

    // -- WP SECAN --
    /**
     * Read the session's encryption ([LinkCoreOps.security]) into [security], and keep the
     * credential the core gives when it differs from the stored one (a v1 token upgraded
     * to a pinned v2 credential on the first TLS connection, or re-pinned after a signed
     * key rotation). The core delivers the same through `onLinkTokenIssued`; this is the
     * "save if changed" for a connect that upgraded without a callback.
     */
    private suspend fun readSecurity(h: LinkHub, gen: Int) {
        val sec = ops.security() ?: return
        if (gen != generation) return
        if (sec.credential.isNotEmpty() && !h.id.startsWith(MANUAL_ID_PREFIX) && tokens.token(h.id) != sec.credential) {
            tokens.setToken(h.id, h.name, sec.credential)
            log("link: the hub's credential was updated")
        }
        val noticed = (h.id.lowercase() + "|" + sec.hubKey) in keysNoticed
        val notice = sec.encrypted && sec.hubKeyNew && sec.hubKey.isNotEmpty() && !noticed
        val next = HubSecurity(h.id, sec.encrypted, sec.hubKey, notice, local = sec.local) // -- WP SECFIX -- local
        if (_security.value != next) _security.value = next
    }

    private fun markInterrupted() {
        if (attached == null || interrupted) return
        interrupted = true
        ops.setInterrupted(true)
    }

    private fun serviceRunning(on: Boolean) {
        if (on == serviceOn) return
        serviceOn = on && service.start()
        if (!serviceOn) {
            // Refused (background, F10), or asked to stop. Refused: the next look at a bound state tries again.
            if (on) log("link: the system refused the foreground service for now")
            service.stop()
        }
    }

    private suspend fun detachCurrent() {
        serviceRunning(false)
        ops.setReadOnly(false)
        ops.setInterrupted(false)
        interrupted = false
        val d = attached ?: return
        attached = null
        runCatching { ops.detach(d.serial) }
    }

    private fun describe(e: DspiException): String = when (e) {
        is DspiException.Link -> e.msg
        else -> e.message ?: e.toString()
    }

    /** macOS NetworkTransport: "Pairing failed: …". */
    private fun pairingError(e: DspiException): String = when {
        e is DspiException.Link && e.code == "wrong_pin" -> "Pairing failed: the hub did not accept that code."
        e is DspiException.Link && e.code == "rate_limited" -> "Pairing failed: too many attempts. Wait a minute and try again."
        else -> "Pairing failed: ${describe(e)}"
    }

    private companion object {
        const val MANUAL_ID_PREFIX = "manual:"

        // -- WP LKFAST -- the status line of a start's dial that found no hub at the stored address
        const val AT_START_NOT_FOUND = "Not at its last address. Looking for it on the network…"

        // -- WP LKFAST3 --
        const val AT_START_NOT_FOUND_NO_SEARCH =
            "Not at its last address. To find it, turn on Find hubs automatically in Settings › Networking."

        /** A dial's codes that mean nothing answered at the address (no hub there now). */
        val NOT_THERE_CODES = setOf("io", "timeout", "handshake")
        const val VIEWER = "viewer"

        /** Close codes in `LinkStatus.closeCode` (spec 5). */
        const val CLOSE_PROTOCOL_ERROR = 4000
        const val CLOSE_REVOKED = 4002

        /**
         * WP AN4. Link is a remote control, nothing plays through it, so ten minutes
         * covers a hub restarting or a walk out of Wi-Fi range. Longer, a hub that is
         * away (a Mac asleep, overnight) would hold the process up with a dial every
         * 30 s and a browse with the multicast lock, for nobody, under a notification.
         */
        const val RECONNECT_SERVICE_BOUND_MS = 10 * 60 * 1000L
    }
}

/**
 * The PIN prompt's text (macOS `PairingPrompt`, spec 6.2): a Console hub
 * shows the code under Settings > Networking; another hub's window PIN is
 * "shown on the hub"; a fixed PIN is the hub's own.
 */
object PairingText {
    fun title(hub: LinkHub) = "Pair with ${hub.name}"

    fun message(hub: LinkHub, pinSource: String): String = when {
        pinSource == "fixed" -> "Enter the hub's PIN."
        hub.kind == "console" ->
            "On ${hub.name}, open Settings > Networking and choose Allow a New Client, then enter the code it shows."
        else -> "Enter the PIN shown on the hub."
    }
}
