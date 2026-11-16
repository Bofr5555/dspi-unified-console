package se.bofr.dspi.link

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.LinkDevice
import se.bofr.dspi.core.LinkHello
import se.bofr.dspi.core.LinkInfo
import se.bofr.dspi.core.LinkListener
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.LinkPhaseText
import se.bofr.dspi.core.LinkStatus

/**
 * What [LinkSessionEngine] needs from the REMOTE stack (narrow, so tests can
 * fake it). Every suspend call may block on the network for seconds.
 */
interface LinkCoreOps {
    /**
     * [expectedHubId]: the stored hub's id, so its token goes to that hub alone; "" when not known yet.
     * [tlsAdvertised]: the browsed record said `tls=1` ([LinkHub.tlsAdvertised]), so TLS must not fall back.
     */
    suspend fun connect(
        url: String,
        clientName: String,
        appVersion: String,
        token: String,
        expectedHubId: String,
        tlsAdvertised: Boolean
    ): LinkHello

    suspend fun pair(pin: String): String

    suspend fun devices(): List<LinkDevice>

    suspend fun status(): LinkStatus

    suspend fun fetchInfo(host: String, port: Int): LinkInfo

    /** End the session, reconnecting included. Does not wait for a stalled link. */
    suspend fun close()

    /** Cut the core's backoff short. A no-op unless reconnecting. */
    suspend fun retryNow()

    /** The hub answers at [url] now: the core reconnects there, at once if reconnecting. */
    suspend fun setUrl(url: String)

    /** Bind the bridge to a device of the open session ([DspiBridge.attachRemote]). */
    suspend fun attach(handle: Long, serial: String): Boolean

    suspend fun detach(serial: String)

    /** After the core re-attached by itself: the analyser's worker ended with the old connection. */
    suspend fun restartAnalyser()

    // -- WP MERGEP1D --
    /**
     * Read the hub's installer lock of the bound device (`installerLockRefresh`), after a bind
     * and after a reconnect, so the core refuses a write in the hub's words (`installer_lock.hub`).
     * Never fails. The one lock prompt (LockState.heldBy) comes with P1-E.
     */
    suspend fun refreshLock() {}

    fun wsUrl(host: String, port: Int, path: String): String

    /** The core's wording for [phase] (CG10). */
    fun phaseText(phase: LinkPhase, pinSource: String): LinkPhaseText

    fun setListener(listener: LinkListener?)

    /** The hub's role is `viewer` ([DspiBridge.setRemoteReadOnly]). */
    fun setReadOnly(readOnly: Boolean)

    /** The device is out of reach for now ([DspiBridge.setRemoteInterrupted]). */
    fun setInterrupted(interrupted: Boolean)

    /** The bridge still has the device open ([DspiBridge.connected]). */
    fun deviceOpen(): Boolean
}

/**
 * [LinkCoreOps] on the REMOTE stack: [core] is the stack's own core, [bridge]
 * its bridge, [ffi] the bridge's serial FFI dispatcher.
 *
 * Session calls run on [network], not on [ffi]: a dial blocks for up to 15 s,
 * and the bridge's own calls must not queue behind it. `linkClose` has to be
 * off [ffi] too, since it is what ends a command stuck there. Binding a device
 * reads it, as the bridge's other device calls do, so that stays on [ffi].
 */
class BridgeLinkCoreOps(
    private val core: DspiCoreApi,
    private val bridge: DspiBridge,
    private val ffi: CoroutineDispatcher,
    private val network: CoroutineDispatcher = Dispatchers.IO
) : LinkCoreOps {
    override suspend fun connect(
        url: String,
        clientName: String,
        appVersion: String,
        token: String,
        expectedHubId: String,
        tlsAdvertised: Boolean
    ): LinkHello = withContext(network) {
        core.linkConnectAdvertised(url, clientName, appVersion, token, expectedHubId, tlsAdvertised)
    }

    override suspend fun pair(pin: String): String = withContext(network) { core.linkPair(pin) }

    override suspend fun devices(): List<LinkDevice> = withContext(network) { core.linkDevices() }

    // The core lock, which a command on a stalled link can hold for seconds: not on the main thread
    override suspend fun status(): LinkStatus = withContext(network) { core.linkStatus() }

    override suspend fun fetchInfo(host: String, port: Int): LinkInfo = withContext(network) { core.linkFetchInfo(host, port) }

    override suspend fun close() = withContext(network) { core.linkClose() }

    override suspend fun retryNow() = withContext(network) { core.linkRetryNow() }

    override suspend fun setUrl(url: String) = withContext(network) { core.linkSetUrl(url) }

    override suspend fun attach(handle: Long, serial: String): Boolean = bridge.attachRemote(handle, serial)

    override suspend fun detach(serial: String) = bridge.detachDevice(serial)

    override suspend fun restartAnalyser() = withContext(ffi) { core.rtaStart() }

    // -- WP MERGEP1D -- a hub request: on [network], as the session's calls are
    override suspend fun refreshLock() {
        withContext(network) { runCatching { core.installerLockRefresh() } }
    }

    override fun wsUrl(host: String, port: Int, path: String): String = core.linkWsUrl(host, port, path)

    override fun phaseText(phase: LinkPhase, pinSource: String): LinkPhaseText = core.linkPhaseText(phase, pinSource)

    override fun setListener(listener: LinkListener?) = core.setLinkListener(listener)

    override fun setReadOnly(readOnly: Boolean) = bridge.setRemoteReadOnly(readOnly)

    override fun setInterrupted(interrupted: Boolean) = bridge.setRemoteInterrupted(interrupted)

    override fun deviceOpen(): Boolean = bridge.connected
}
