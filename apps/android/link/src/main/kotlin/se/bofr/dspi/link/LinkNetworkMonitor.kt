// WP AN4: what the platform's network and the hub's DNS-SD announcements mean for a
// session. Everything here is an edge handed on once: no timer, no retry loop, and
// nothing that writes. The core still does the reconnecting (WP C15).
package se.bofr.dspi.link

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.Looper
import android.util.Log
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine

/** Who hears about the network: the session engine. Main thread. */
interface LinkNetworkTarget {
    /** A local network came up. */
    fun networkAvailable()

    /** A local network went away. */
    fun networkLost()

    /** DNS-SD announces the wanted hub, newly or at another address. */
    fun hubSeen(found: LinkHub)
}

/**
 * Turns the callbacks of [ConnectivityManager.NetworkCallback] into edges and
 * decides the process's network binding. Pure, for the tests; main thread.
 *
 * A network is an edge once, when it is new: the callback repeats itself in
 * `onCapabilitiesChanged` (signal strength) and never says anything to act on
 * there. Registering replays the networks already up as `available`. That is
 * harmless: the monitor registers when Link is turned on, before there is a
 * session to redial.
 *
 * **Binding (F10, Wi-Fi without internet):** Android makes cellular the default
 * network when Wi-Fi has no validated internet, and an unbound socket then cannot
 * reach a hub on that Wi-Fi. So while a session is wanted and no local network is
 * validated, the process is bound to the newest local network. When one validates
 * the default route already goes there, and the binding is lifted.
 */
class LinkNetworkEdges(private val bind: (Long?) -> Unit, private val onAvailable: () -> Unit, private val onLost: () -> Unit) {
    /** Up now, oldest first: network handle to "validated". */
    private val networks = LinkedHashMap<Long, Boolean>()
    private var sessionActive = false
    private var bound: Long? = null

    fun available(id: Long) {
        if (id in networks) return
        networks[id] = false
        rebind() // first: the dial this edge starts must use it
        onAvailable()
    }

    fun lost(id: Long) {
        if (networks.remove(id) == null) return
        rebind()
        onLost()
    }

    fun validated(id: Long, validated: Boolean) {
        if (id !in networks || networks[id] == validated) return
        networks[id] = validated
        rebind()
    }

    fun setSessionActive(active: Boolean) {
        sessionActive = active
        rebind()
    }

    /** The callback is gone: nothing is known any more, and nothing stays bound. */
    fun reset() {
        networks.clear()
        rebind()
    }

    private fun rebind() {
        val want = if (sessionActive && networks.values.none { it }) networks.keys.lastOrNull() else null
        if (want == bound) return
        bound = want
        bind(want)
    }
}

/**
 * [LinkNetworkEdges] on the platform. Calls [onAvailable] when a local network
 * (Wi-Fi or Ethernet) comes up and [onLost] when one goes, on the main thread (the
 * Mac hub vanishes when the Mac sleeps; the phone drops Wi-Fi in its pocket).
 * Needs ACCESS_NETWORK_STATE; `bindProcessToNetwork` needs no permission.
 *
 * Main thread.
 */
class LinkNetworkMonitor(context: Context, onAvailable: () -> Unit, onLost: () -> Unit) {
    private val cm = context.applicationContext.getSystemService(ConnectivityManager::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private var callback: ConnectivityManager.NetworkCallback? = null
    private val known = HashMap<Long, Network>()
    private val edges = LinkNetworkEdges(::bind, onAvailable, onLost)

    fun start() {
        if (callback != null || cm == null) return
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                known[network.networkHandle] = network
                edges.available(network.networkHandle)
            }

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) =
                edges.validated(network.networkHandle, caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED))

            override fun onLost(network: Network) {
                edges.lost(network.networkHandle)
                known.remove(network.networkHandle)
            }
        }
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
            .build()
        runCatching { cm.registerNetworkCallback(request, cb, handler) }.onSuccess { callback = cb }
    }

    fun stop() {
        callback?.let { runCatching { cm?.unregisterNetworkCallback(it) } }
        callback = null
        edges.reset()
        known.clear()
    }

    /** A REMOTE stack exists. Set before its first dial, which the binding must already apply to. */
    fun setSessionActive(active: Boolean) = edges.setSessionActive(active)

    private fun bind(id: Long?) {
        val ok = runCatching { cm?.bindProcessToNetwork(id?.let(known::get)) }.getOrNull()
        if (ok != true) Log.w(TAG, "could not ${if (id == null) "lift the network binding" else "bind to the local network"}")
    }

    private companion object {
        const val TAG = "DspiLink"
    }
}

/**
 * Whether the session needs DNS-SD now, the Networking page aside. A hidden app
 * looks only for a session the service still holds on to: multicast is what the
 * Wi-Fi chip wakes the phone for.
 */
fun linkBrowseWanted(state: LinkSessionState, uiVisible: Boolean): Boolean = when (state) {
    is LinkSessionState.Reconnecting -> !state.parked
    is LinkSessionState.Failed, is LinkSessionState.Connecting -> uiVisible
    else -> false
}

/**
 * The edges in a hub list: the wanted hub appearing, or showing up at another
 * address. Being still there is no edge, so a dial that fails while the hub stays
 * listed is not repeated.
 */
class HubSightings {
    private var wanted: String? = null
    private var seenAt: Triple<String, Int, String>? = null

    /** What to report for this list, if anything. */
    fun next(wantedId: String?, hubs: List<LinkHub>): LinkHub? {
        if (!wantedId.equals(wanted, ignoreCase = true)) {
            wanted = wantedId
            seenAt = null
        }
        val found = wantedId?.let { id -> hubs.firstOrNull { it.id.equals(id, ignoreCase = true) } }
        val at = found?.let { Triple(it.host, it.port, it.path) }
        val edge = at != null && at != seenAt
        seenAt = at
        return found.takeIf { edge }
    }
}

/**
 * Hands the network's and the browser's edges to the session, and looks again
 * when the answer may have changed: NsdManager does not report a known service
 * again when only its address changed, so a browse already running would keep
 * the old address. Main thread.
 */
class LinkNetworkFollower(
    private val browser: HubBrowser,
    private val state: StateFlow<LinkSessionState>,
    private val target: () -> LinkNetworkTarget?
) {
    private val sightings = HubSightings()
    private var reconnecting = false

    fun networkAvailable() {
        target()?.networkAvailable()
        lookAgain()
    }

    fun networkLost() {
        target()?.networkLost()
    }

    /** The app came back in front. */
    fun uiShown() = lookAgain()

    /** Follows the browser and the session until cancelled. */
    suspend fun follow() {
        combine(browser.hubs, state, ::Pair).collect { (hubs, st) ->
            val wasReconnecting = reconnecting
            reconnecting = st is LinkSessionState.Reconnecting
            if (reconnecting && !wasReconnecting) browser.refresh()
            sightings.next(st.hub?.id, hubs)?.let { target()?.hubSeen(it) }
        }
    }

    private fun lookAgain() {
        val st = state.value
        if (st is LinkSessionState.Reconnecting || st is LinkSessionState.Failed) browser.refresh()
    }
}
