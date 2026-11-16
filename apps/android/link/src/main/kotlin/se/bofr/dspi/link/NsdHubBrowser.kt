// Android counterpart of DSPi-Console release/v1.1.6 `Link/HubBrowser.swift`
// (NWBrowser over `_dspi._tcp`, resolve each result, keep the TXT keys):
// NsdManager discovery, one resolve at a time (NsdManager refuses concurrent
// resolves before API 34), IPv4 preferred (an IPv6 link-local address needs a
// zone the WebSocket URL cannot carry).
package se.bofr.dspi.link

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import java.net.Inet4Address
import java.net.InetAddress
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.core.LinkTxtEntry
import se.bofr.dspi.core.LinkTxtRecord

/** Discovered hubs (DNS-SD). Main thread for start/stop; state from any thread. */
interface HubBrowser {
    val hubs: StateFlow<List<LinkHub>>

    /** Non-null when browsing failed ("Local network unavailable", ...). */
    val error: StateFlow<String?>

    val browsing: StateFlow<Boolean>

    fun start()

    fun stop()

    /** Forget what is known and look afresh, if browsing is on (WP AN4). */
    fun refresh() {}
}

/** [parseTxt]: the core's `linkParseTxt`, the one TXT reader (SEC-B). */
class NsdHubBrowser(
    context: Context,
    private val scope: CoroutineScope,
    private val parseTxt: (String, List<LinkTxtEntry>) -> LinkTxtRecord
) : HubBrowser {
    private val appContext = context.applicationContext
    private val nsd = appContext.getSystemService(NsdManager::class.java)
    private val wifi = appContext.getSystemService(WifiManager::class.java)
    private var multicast: WifiManager.MulticastLock? = null

    private val _hubs = MutableStateFlow<List<LinkHub>>(emptyList())
    override val hubs: StateFlow<List<LinkHub>> = _hubs.asStateFlow()
    private val _error = MutableStateFlow<String?>(null)
    override val error: StateFlow<String?> = _error.asStateFlow()
    private val _browsing = MutableStateFlow(false)
    override val browsing: StateFlow<Boolean> = _browsing.asStateFlow()

    /** Resolved hubs by service name. */
    private val found = LinkedHashMap<String, LinkHub>()
    private val queue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false
    private var listener: NsdManager.DiscoveryListener? = null

    /** [start] was called and [stop] was not; a failed start is still wanted. */
    private var wanted = false

    // WP DISC2: a lost hub stays listed a little, in case it is found again at once.
    private val grace = RemovalGrace(scope) { name ->
        found.remove(name)
        publish()
    }

    override fun start() {
        wanted = true
        if (listener != null || nsd == null) return
        // Some Wi-Fi drivers drop multicast while the screen is off unless a lock is held
        multicast = runCatching {
            wifi?.createMulticastLock("dspi-link")?.apply {
                setReferenceCounted(false)
                acquire()
            }
        }.getOrNull()
        val l = object : NsdManager.DiscoveryListener {
            // A listener replaced by refresh() may still report: only the current one counts
            private fun current(block: () -> Unit) = post { if (listener === this) block() }

            override fun onDiscoveryStarted(serviceType: String) = current {
                _browsing.value = true
                _error.value = null
            }

            override fun onDiscoveryStopped(serviceType: String) = current { _browsing.value = false }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) = current {
                _browsing.value = false
                _error.value = "Could not look for hubs (error $errorCode)."
                listener = null
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit

            override fun onServiceFound(info: NsdServiceInfo) = current {
                // WP DISC2: back within the grace, so it stays; resolved anew for new data.
                grace.back(info.serviceName)
                queue.addLast(info)
                resolveNext()
            }

            override fun onServiceLost(info: NsdServiceInfo) = current {
                // WP DISC2: not at once (RemovalGrace).
                if (found.containsKey(info.serviceName)) grace.lost(info.serviceName)
            }
        }
        listener = l
        try {
            nsd.discoverServices(LINK_SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, l)
        } catch (e: RuntimeException) {
            Log.w(TAG, "discoverServices: $e")
            listener = null
            _error.value = "Could not look for hubs."
        }
    }

    override fun stop() {
        wanted = false
        listener?.let { runCatching { nsd?.stopServiceDiscovery(it) } }
        listener = null
        runCatching { multicast?.release() }
        multicast = null
        queue.clear()
        found.clear()
        grace.clear() // WP DISC2
        publish()
        _browsing.value = false
    }

    override fun refresh() {
        if (!wanted) return
        stop()
        start()
    }

    private fun post(block: () -> Unit) {
        scope.launch { block() }
    }

    private fun publish() {
        _hubs.value = found.values.sortedBy { it.name.lowercase() }
    }

    @Suppress("DEPRECATION") // resolveService: the API 34 ServiceInfoCallback is not available below 34
    private fun resolveNext() {
        if (resolving || listener == null) return
        val next = queue.removeFirstOrNull() ?: return
        resolving = true
        val cb = object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = post {
                resolving = false
                resolveNext()
            }

            override fun onServiceResolved(info: NsdServiceInfo) = post {
                resolving = false
                if (listener != null) {
                    val host = addressOf(info)
                    val hub = host?.let { HubTxt.parse(info.serviceName, info.attributes.orEmpty(), it, info.port, parseTxt) }
                    if (hub != null) {
                        found[info.serviceName] = hub
                        publish()
                    }
                }
                resolveNext()
            }
        }
        try {
            nsd?.resolveService(next, cb)
        } catch (e: RuntimeException) {
            resolving = false
            Log.w(TAG, "resolveService: $e")
        }
    }

    @Suppress("DEPRECATION")
    private fun addressOf(info: NsdServiceInfo): String? {
        val all: List<InetAddress> = if (Build.VERSION.SDK_INT >= 34) info.hostAddresses else listOfNotNull(info.host)
        val pick = all.firstOrNull { it is Inet4Address } ?: all.firstOrNull { !it.isLinkLocalAddress } ?: return null
        return pick.hostAddress?.substringBefore('%')
    }

    private companion object {
        const val TAG = "DspiLink"
    }
}
