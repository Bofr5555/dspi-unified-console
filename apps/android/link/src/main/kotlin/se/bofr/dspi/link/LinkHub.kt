// Port of DSPi-Console release/v1.1.6 `Link/HubBrowser.swift` `DiscoveredHub`
// (the fields the client uses) and the DNS-SD TXT keys of DSPi-Link spec 3.1.
package se.bofr.dspi.link

import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkTxtEntry
import se.bofr.dspi.core.LinkTxtRecord

/** DNS-SD service type of a DSPi Link hub (spec 3.1). */
const val LINK_SERVICE_TYPE = "_dspi._tcp"

/** Default hub port: decimal of the USB VID 0x2E8B (spec 3.1). */
const val LINK_DEFAULT_PORT = 11915

const val LINK_DEFAULT_PATH = "/dspi/v1"

/** A hub found by DNS-SD or added by address (macOS `DiscoveredHub`). */
data class LinkHub(
    /** `hid` (lowercase UUID); `manual:host:port` for a typed address until its hello names it. */
    val id: String,
    val name: String,
    /** `console`, `bridge` or `unknown`. */
    val kind: String,
    /** `none` (open) or `pin` (private). */
    val auth: String,
    val deviceCount: Int,
    /** TXT `d`: serials of the shared devices, when the hub lists them. */
    val serials: List<String>,
    val host: String,
    val port: Int,
    val path: String = LINK_DEFAULT_PATH,
    val isManual: Boolean = false,
    /**
     * SEC-B: the TXT record said `tls=1`. A hint, not trust (anyone on the LAN can
     * forge or strip it), passed to the connect so advertised TLS never falls back
     * to plaintext. False for an upstream hub and for a typed address.
     */
    val tlsAdvertised: Boolean = false
) {
    /** "192.168.1.20" or "studio.local:8080" (the port only when it is not the default). */
    val displayAddress: String
        get() {
            val h = if (':' in host) "[$host]" else host
            return if (port == LINK_DEFAULT_PORT) h else "$h:$port"
        }

    /** Hubs that answer to `auth: none` (macOS badge "Open"; else "Private"). */
    val isOpen: Boolean get() = auth == "none"

    companion object {
        fun manualId(host: String, port: Int) = "manual:$host:$port"

        /** A typed address (already parsed); resolved to a real id on hello. */
        fun manual(host: String, port: Int) =
            LinkHub(manualId(host, port), host, "unknown", "pin", 0, emptyList(), host, port, isManual = true)
    }
}

/**
 * TXT record of a browsed hub (spec 3.1), read by the core's one parser
 * (`link_parse_txt`, the desktop browser's rules): `v` and `hid` required, unknown
 * keys ignored, `tls=1` read into [LinkHub.tlsAdvertised], and DNS-SD's `\DDD`
 * escapes in the instance name undone.
 */
object HubTxt {
    /** [attrs] as `NsdServiceInfo.attributes` gives them; a key without a value reads as "". */
    fun entries(attrs: Map<String, ByteArray?>): List<LinkTxtEntry> =
        attrs.map { (k, v) -> LinkTxtEntry(k, v?.toString(Charsets.US_ASCII).orEmpty()) }

    /**
     * The hub at [host]:[port] that [serviceName]'s TXT [attrs] describe, or null
     * when [parseTxt] (the core's `linkParseTxt`) says it is no hub.
     */
    fun parse(
        serviceName: String,
        attrs: Map<String, ByteArray?>,
        host: String,
        port: Int,
        parseTxt: (String, List<LinkTxtEntry>) -> LinkTxtRecord
    ): LinkHub? {
        val r = try {
            parseTxt(serviceName, entries(attrs))
        } catch (e: DspiException) {
            return null
        }
        return LinkHub(
            id = r.hubId,
            name = r.name,
            kind = r.kind,
            auth = r.auth,
            deviceCount = r.deviceCount.coerceIn(0, Int.MAX_VALUE.toLong()).toInt(),
            serials = r.serials,
            host = host,
            port = port,
            path = r.path,
            tlsAdvertised = r.tlsAdvertised
        )
    }
}
