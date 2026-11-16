// -- WP SECAN -- The REMOTE session's encryption as the UI shows it (PLAN-SEC 4.3):
// the core's `link_security`, the codes that offer pairing again, and the badges.
package se.bofr.dspi.link

/** What the UI shows about the current hub connection's encryption. */
data class HubSecurity(
    /** The hub this is about ([LinkSessionState.hub]'s id). */
    val hubId: String,
    /**
     * TLS with a verified hub key. False: unencrypted (a hub without TLS, an upstream
     * hub, a hub in compatibility mode): the connection bar is red and says "(unsecured)".
     */
    val encrypted: Boolean,
    /** The hub key's fingerprint (`SHA256:K7Q2-M9XD-4TNB-0ZRA`), "" when none. */
    val hubKey: String,
    /** Show "Encrypted connection set up; hub key …" now; once per hub and key ([LinkSessionEngine.ackKeyNotice]). */
    val keyNotice: Boolean = false,
    /**
     * -- WP SECFIX -- Not [encrypted], but the hub is this device (the core's `local`: 127.0.0.1,
     * ::1, localhost, the same rule as every client): nothing crosses a network, so the bar is not
     * red and says nothing.
     */
    val local: Boolean = false
) {
    /** -- WP SECFIX -- The bar is red and says "(unsecured)": neither encrypted nor local. */
    val unsecured: Boolean get() = !encrypted && !local
}

/**
 * The `DspiException.Link` codes after which the app offers pairing again, and the only
 * ones (PLAN-SEC 4.3): nothing was sent to the hub, and there is no "connect anyway".
 */
val REPAIR_CODES: Set<String> = setOf("hub_key_changed", "hub_key_unverified", "tls_refused", "tls_required")

/** A hub list row's encryption badge. */
enum class HubBadge(val label: String) {
    /** Paired, and its key is pinned: every connection checks it. */
    VERIFIED("Verified"),

    /** Not paired over TLS yet, but its record says it serves TLS. */
    ENCRYPTED("Encrypted"),

    /** Neither: an upstream hub, TLS off, or paired before TLS and not met since. */
    UNENCRYPTED("Unencrypted"),

    /** A typed address with nothing known yet. */
    UNKNOWN("")
}

/**
 * The badge of [hub]: [pinned] is whether its stored credential carries a key pin
 * (`link_credential_hub_key` not empty). The advert is a hint only; the pin is what
 * the core checks.
 */
fun hubBadge(hub: LinkHub, pinned: Boolean): HubBadge = when {
    pinned -> HubBadge.VERIFIED
    hub.tlsAdvertised -> HubBadge.ENCRYPTED
    hub.isManual -> HubBadge.UNKNOWN
    else -> HubBadge.UNENCRYPTED
}
