// -- WP ANSEC -- (owner 2026-11-21) The user's acknowledgement that a hub's DSPi Link
// connection is unsecured: the bar then keeps its normal colours, but still says
// "(unsecured)" and keeps its info button. Remembered per hub across restarts, dropped
// when the hub's identity or security changes, and forgotten in Settings › Networking.
package se.bofr.dspi.link

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * One acknowledged hub: [hubId] (lowercase), and for a typed hub ([LinkHub.isManual]) the
 * address it was typed as, so a different hub answering there drops it. [name] is for
 * Settings › Networking only.
 */
data class UnsecuredAck(val hubId: String, val name: String, val host: String = "", val port: Int = 0) {
    val typed: Boolean get() = host.isNotEmpty()

    /** [hub] is the hub this was given for: the same id, and when both were typed, the same address. */
    fun covers(hub: LinkHub): Boolean = hub.id.equals(hubId, ignoreCase = true) && (!typed || !hub.isManual || at(hub))

    fun at(hub: LinkHub): Boolean = host == hub.host && port == hub.port
}

/**
 * The acknowledgements over [KeyValue] (a private SharedPreferences file in the app, a map in
 * tests): per hub `v1\n<host>\n<port>\n<name>`. Main thread, as the mode controller.
 */
class UnsecuredAckStore(private val kv: KeyValue) {
    private val _acks = MutableStateFlow(load())

    /** Every acknowledged hub, by name. */
    val acks: StateFlow<List<UnsecuredAck>> = _acks.asStateFlow()

    private fun key(hubId: String) = PREFIX + hubId.lowercase()

    private fun load(): List<UnsecuredAck> = kv.keys().filter { it.startsWith(PREFIX) }.mapNotNull { k ->
        val parts = kv.get(k)?.split('\n', limit = 4) ?: return@mapNotNull null
        if (parts.size != 4 || parts[0] != "v1") return@mapNotNull null
        UnsecuredAck(k.removePrefix(PREFIX), parts[3], parts[1], parts[2].toIntOrNull() ?: 0)
    }.sortedBy { it.name.lowercase() }

    private fun reload() {
        _acks.value = load()
    }

    /** The bar of an unsecured connection to [hub] is not highlighted. */
    fun acknowledged(hub: LinkHub): Boolean = _acks.value.any { it.covers(hub) }

    /** The user acknowledged that the connection to [hub] is unsecured. */
    fun acknowledge(hub: LinkHub) {
        val host = if (hub.isManual) hub.host else ""
        val port = if (hub.isManual) hub.port else 0
        kv.put(key(hub.id), listOf("v1", host, port.toString(), hub.name.replace('\n', ' ')).joinToString("\n"))
        reload()
    }

    /**
     * A live connection to [hub] with [security]: drop what no longer holds. An encrypted (or
     * local) connection drops the hub's acknowledgement, so a later unsecured one is a change and
     * highlighted again; a typed address where another hub now answers drops the old one's.
     * True when something was dropped.
     */
    fun observe(hub: LinkHub, security: HubSecurity): Boolean {
        val stale = _acks.value.filter { a ->
            val same = hub.id.equals(a.hubId, ignoreCase = true)
            (same && !security.unsecured) || (!same && a.typed && hub.isManual && a.at(hub))
        }
        stale.forEach { kv.put(key(it.hubId), null) }
        if (stale.isNotEmpty()) reload()
        return stale.isNotEmpty()
    }

    fun forget(hubId: String) {
        kv.put(key(hubId), null)
        reload()
    }

    fun forgetAll() {
        kv.keys().filter { it.startsWith(PREFIX) }.forEach { kv.put(it, null) }
        reload()
    }

    private companion object {
        const val PREFIX = "ack:"
    }
}

/** [KeyValue] in memory: tests, and a controller built without the app's store. */
class InMemoryKeyValue : KeyValue {
    private val map = mutableMapOf<String, String>()

    override fun get(key: String): String? = map[key]

    override fun put(key: String, value: String?) {
        if (value == null) map.remove(key) else map[key] = value
    }

    override fun keys(): Set<String> = map.keys.toSet()
}
