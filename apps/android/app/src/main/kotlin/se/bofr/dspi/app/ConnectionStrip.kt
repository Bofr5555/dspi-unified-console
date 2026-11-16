package se.bofr.dspi.app

import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.LinkPhaseText
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.link.HubSecurity
import se.bofr.dspi.link.LinkHub
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.usb.UsbSessionState

/** What the strip under the top bar offers; pure so it is JVM-tested. */
sealed interface ConnectionStrip {
    val text: String

    /**
     * -- WP AN42 -- the one line a narrow strip shows (phones); [text], the whole of it, is
     * then its long-press tooltip and what TalkBack reads. Wide strips show [text].
     */
    val short: String get() = text

    /**
     * WP AN7: the USB DSPi is open. HOME shows the connection and the compat banner; the
     * strip stays, slim, because it holds the device list (demo and DSPi Link with a DSPi attached).
     */
    data object Usb : ConnectionStrip {
        override val text = "DSPi on USB"
    }

    /** No DSPi attached; demo and DSPi Link are in the device list ([deviceList]). */
    data object NoDevice : ConnectionStrip {
        override val text = "No DSPi connected. Plug one in over USB."
        override val short = "No DSPi connected"
    }

    /** A DSPi is attached but may not be opened: offer the permission prompt. */
    data class NeedsPermission(override val text: String, override val short: String = text) : ConnectionStrip

    data class Info(override val text: String, override val short: String = text) : ConnectionStrip

    /** Demo mode is on: always visible, always labelled, with the way out. */
    data object Demo : ConnectionStrip {
        override val text = "Demo: a simulated DSPi. Nothing is sent to a real DSPi."
        override val short = "Demo: a simulated DSPi"
    }

    // -- WP P1AN1 --
    /** A document opened offline (P1OF): "File: <name>" in place of a device, with the way out. */
    data class Document(val fileName: String) : ConnectionStrip {
        override val text = "File: $fileName. Nothing is sent to a DSPi."
        override val short = "File: $fileName"
    }

    /**
     * REMOTE (DSPi Link): always visible, so a remote unit is never mistaken for the USB one.
     * [lost] (WP AN5): the connection is lost and the core is reconnecting, shown as the
     * owner's banner; [detail], the banner's second line, is the hub and the core's `close_reason`.
     * -- WP SECAN -- [unsecured]: the hub connection is not encrypted (owner rule 2026-11-15):
     * the bar is red, [text] ends in " (unsecured)" (so it is never colour alone) and an
     * info button beside it explains ([UnsecuredWording.info]).
     */
    data class Remote(
        override val text: String,
        val actions: List<RemoteAction>,
        val lost: Boolean = false,
        val detail: String = "",
        val unsecured: Boolean = false,
        /** " (unsecured)" when [unsecured]: the end of [text], which a narrow bar never cuts off. */
        val suffix: String = ""
    ) : ConnectionStrip
}

/** [DISCONNECT]'s label is CG10's `reconnecting.disconnect` (a test holds them equal). */
enum class RemoteAction(val label: String) {
    RETRY("Retry now"),
    CONNECT("Connect"),
    DISCONNECT("Disconnect")
}

// -- WP AN29 --
/** Android keeps a USB grant only until unplug; the "Always open" tick is what persists it. */
const val USB_ALWAYS_OPEN_HINT = "Tick \"Always open\" in Android's dialog and it will not ask again."

// -- WP AN5 --
/**
 * CG10's wording for a [LinkPhase] (`link_phase_text`): the one source of the
 * Link status text. [pinSource] matters only to [LinkPhase.NEEDS_PAIRING].
 */
fun interface LinkWording {
    fun of(phase: LinkPhase, pinSource: String): LinkPhaseText

    companion object {
        /** The core's table. It never changes, so it is read once; no core instance is needed. */
        val CORE: LinkWording = object : LinkWording {
            private val table: Map<Pair<LinkPhase, Boolean>, LinkPhaseText> by lazy {
                LinkPhase.entries.flatMap { p -> listOf(false, true).map { fixed -> (p to fixed) to coreText(p, fixed) } }.toMap()
            }

            override fun of(phase: LinkPhase, pinSource: String) = table.getValue(phase to (pinSource == FIXED_PIN))
        }

        private fun coreText(phase: LinkPhase, fixed: Boolean): LinkPhaseText {
            val t = se.bofr.dspi.ffi.uniffi.linkPhaseText(
                se.bofr.dspi.ffi.uniffi.LinkPhase.valueOf(phase.name),
                if (fixed) FIXED_PIN else ""
            )
            return LinkPhaseText(t.key, t.phase, t.text, t.actionKey, t.action)
        }
    }
}

/** `pin_source` of a hub with a PIN of its own; anything else is a window PIN (spec 6.2). */
const val FIXED_PIN = "fixed"

// -- WP SECAN --
/** The core's notice row of the unsecured bar: its title is the suffix's word. */
const val UNSECURED_NOTICE_KEY = "dlink.unsecured"

/** The core's notice row the info button beside "(unsecured)" opens: calm, what it means, how to turn TLS on. */
const val UNSECURED_INFO_NOTICE_KEY = "dlink.unsecured.info"

/** The shared words of the unsecured bar (the core's notice table, every client the same). */
fun interface UnsecuredWording {
    fun notice(key: String): Notice

    /** " (unsecured)", appended to the bar's text. */
    fun suffix(): String = " (" + notice(UNSECURED_NOTICE_KEY).title + ")"

    /** The info button's explanation. */
    fun info(): Notice = notice(UNSECURED_INFO_NOTICE_KEY)

    companion object {
        /** The core's table; no core instance needed (as [LinkWording.CORE]). */
        val CORE: UnsecuredWording = object : UnsecuredWording {
            private val rows = mutableMapOf<String, Notice>()

            override fun notice(key: String): Notice = synchronized(rows) {
                rows.getOrPut(key) {
                    val n = se.bofr.dspi.ffi.uniffi.noticeForKey(key, "")
                    Notice(n.key, NoticeKind.valueOf(n.kind.name), n.title, n.reason, n.action, n.detail)
                }
            }
        }
    }
}

fun connectionStrip(
    mode: AppMode,
    state: UsbSessionState,
    remote: LinkSessionState = LinkSessionState.Idle,
    lockNotice: Boolean = false,
    wording: LinkWording = LinkWording.CORE,
    // -- WP P1AN1 -- the demo stack holds a document opened offline
    document: String? = null,
    // -- WP SECAN --
    security: HubSecurity? = null,
    unsecuredWording: UnsecuredWording = UnsecuredWording.CORE
): ConnectionStrip = when {
    mode == AppMode.DEMO && document != null -> ConnectionStrip.Document(document)
    mode == AppMode.DEMO -> ConnectionStrip.Demo
    mode == AppMode.REMOTE -> remoteStrip(remote, lockNotice, wording).secured(remote, security, unsecuredWording)
    else -> when (state) {
        UsbSessionState.NoDevice -> ConnectionStrip.NoDevice
        is UsbSessionState.PermissionRequired ->
            ConnectionStrip.NeedsPermission(
                "A DSPi is connected. Allow USB access to use it. $USB_ALWAYS_OPEN_HINT",
                short = "DSPi needs USB access"
            )
        is UsbSessionState.PermissionDenied -> ConnectionStrip.NeedsPermission(
            "USB access to the DSPi was denied.",
            short = "USB access denied"
        )
        // queued prompts are shown by the session as soon as a UI is visible: no second request
        is UsbSessionState.PermissionPending ->
            ConnectionStrip.Info("Waiting for USB permission… $USB_ALWAYS_OPEN_HINT", short = "Waiting for USB permission…")
        is UsbSessionState.Failed -> ConnectionStrip.Info("Could not open the DSPi: ${state.reason}", short = "Could not open the DSPi")
        is UsbSessionState.Open -> ConnectionStrip.Usb
    }
}

// -- WP AN7 --
/** One way to a DSPi in the strip's device list (M10). */
sealed interface DeviceChoice {
    /** The DSPi on this phone's USB (AppMode.DEVICE). */
    data object Usb : DeviceChoice

    /** A hub on the network (DSPi Link). */
    data class Hub(val hub: LinkHub) : DeviceChoice

    /** Opens the Networking dialog to find or type a hub; not a switch by itself. */
    data object Network : DeviceChoice

    /** The core's demo device, offered with or without a DSPi attached. */
    data object Demo : DeviceChoice
}

data class DeviceRow(val choice: DeviceChoice, val label: String, val current: Boolean = false, val enabled: Boolean = true)

/** Whether [remote] is on [hub]: by id, or by address for a typed hub (as Settings › Networking marks it). */
fun onHub(remote: LinkSessionState, hub: LinkHub): Boolean = remote != LinkSessionState.Idle &&
    remote.hub?.let { it.id == hub.id || (hub.isManual && it.host == hub.host && it.port == hub.port) } == true

/**
 * USB, DSPi Link and demo in one list, the current one marked. [hubs] are the hubs
 * Networking knows (browsed and typed); they and "DSPi Link…" are listed only while
 * DSPi Link is on ([linkEnabled]). USB with no DSPi on the bus is listed, but disabled.
 */
fun deviceList(
    mode: AppMode,
    usb: UsbSessionState,
    remote: LinkSessionState = LinkSessionState.Idle,
    hubs: List<LinkHub> = emptyList(),
    linkEnabled: Boolean = false
): List<DeviceRow> = buildList {
    val usbLabel = when (usb) {
        UsbSessionState.NoDevice -> "USB: no DSPi connected"
        is UsbSessionState.Open -> "DSPi on USB"
        else -> "DSPi on USB (not open)"
    }
    add(DeviceRow(DeviceChoice.Usb, usbLabel, current = mode == AppMode.DEVICE, enabled = usb != UsbSessionState.NoDevice))
    if (linkEnabled) {
        hubs.forEach { h ->
            add(DeviceRow(DeviceChoice.Hub(h), "${h.name} (network)", current = mode == AppMode.REMOTE && onHub(remote, h)))
        }
        add(DeviceRow(DeviceChoice.Network, "DSPi Link…"))
    }
    add(DeviceRow(DeviceChoice.Demo, "Demo (no device)", current = mode == AppMode.DEMO))
}

/**
 * The REMOTE strip. The phases the core names say what CG10 says
 * ([LinkWording]), with the hub's name added; a lost connection is the owner's
 * banner ([ConnectionStrip.Remote.lost]). Where the core has no phase the macOS
 * Console's wording stays: a remote device is "<name> via <hub>"
 * (`DSPiDevice.displayName`), a refused command "Another console is applying
 * changes" (`DeviceLock.lockedMessage`).
 */
fun remoteStrip(s: LinkSessionState, lockNotice: Boolean = false, wording: LinkWording = LinkWording.CORE): ConnectionStrip.Remote {
    val d = RemoteAction.DISCONNECT
    fun said(phase: LinkPhase, pinSource: String = "") = wording.of(phase, pinSource).text
    fun on(hub: String, phase: LinkPhase, pinSource: String = "") = ConnectionStrip.Remote("${said(phase, pinSource)} · $hub", listOf(d))
    return when (s) {
        LinkSessionState.Idle -> ConnectionStrip.Remote(said(LinkPhase.IDLE), listOf(d))
        is LinkSessionState.Connecting -> ConnectionStrip.Remote("Connecting to ${s.hub.name}…", listOf(d))
        is LinkSessionState.NeedsPin -> on(s.hub.name, LinkPhase.NEEDS_PAIRING, s.pinSource)
        is LinkSessionState.ChooseDevice -> on(s.hub.name, LinkPhase.CHOOSE_DEVICE)
        // F9: a hub with no board to offer is connected, not failed
        is LinkSessionState.WaitingForDevice -> if (s.serial.isEmpty()) {
            on(s.hub.name, LinkPhase.NO_DEVICE)
        } else {
            ConnectionStrip.Remote("Waiting for DSPi ${s.serial.takeLast(8)} on ${s.hub.name}…", listOf(d))
        }
        is LinkSessionState.Connected -> {
            val name = s.device.name.ifEmpty { "DSPi (${s.device.serial.takeLast(8)})" }
            val text = when {
                lockNotice -> DspiException.LOCKED_MESSAGE
                s.role == "viewer" -> "$name via ${s.hub.name} · view only"
                else -> "$name via ${s.hub.name}"
            }
            ConnectionStrip.Remote(text, listOf(d))
        }
        // The owner's rule: at once, with the way out. The engine's text is the core's (AN3)
        is LinkSessionState.Reconnecting -> ConnectionStrip.Remote(
            s.text,
            listOf(RemoteAction.RETRY, d),
            lost = true,
            detail = if (s.reason.isEmpty()) s.hub.name else "${s.hub.name}: ${s.reason}"
        )
        is LinkSessionState.Failed -> ConnectionStrip.Remote(s.reason, listOf(RemoteAction.CONNECT, d))
    }
}

// -- WP SECAN --
/**
 * The owner's rule (2026-11-15): while the app is on a hub over an unencrypted connection
 * ([HubSecurity.unsecured]: not encrypted, and not this device's own loopback, -- WP SECFIX --),
 * the bar says so: red, " (unsecured)" appended. Only the
 * states with a live hub connection (the PIN prompt, the device choice, waiting for a
 * board, connected); a lost or failed connection has its own banner.
 */
fun ConnectionStrip.Remote.secured(
    s: LinkSessionState,
    security: HubSecurity?,
    wording: UnsecuredWording = UnsecuredWording.CORE
): ConnectionStrip.Remote {
    val live = s is LinkSessionState.NeedsPin ||
        s is LinkSessionState.ChooseDevice ||
        s is LinkSessionState.WaitingForDevice ||
        s is LinkSessionState.Connected
    val hub = s.hub ?: return this
    // -- WP SECFIX -- a hub on this device is local: not unsecured
    if (!live || security == null || !security.unsecured || !security.hubId.equals(hub.id, ignoreCase = true)) return this
    val suffix = wording.suffix()
    return copy(text = text + suffix, unsecured = true, suffix = suffix)
}
