package se.bofr.dspi.app

import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkPhase
import se.bofr.dspi.core.LinkPhaseText
import se.bofr.dspi.link.LinkSessionState
import se.bofr.dspi.usb.UsbSessionState

/** What the strip under the top bar offers; pure so it is JVM-tested. */
sealed interface ConnectionStrip {
    val text: String

    /** Open device: the strip is hidden (HOME shows the connection and the compat banner). */
    data object Hidden : ConnectionStrip {
        override val text = ""
    }

    /**
     * No DSPi attached; [demoAvailable] adds the "Demo (no device)" action,
     * [networkAvailable] (DSPi Link on) "Connect over network".
     */
    data class NoDevice(val demoAvailable: Boolean, val networkAvailable: Boolean = false) : ConnectionStrip {
        override val text = "No DSPi connected. Plug one in over USB."
    }

    /** A DSPi is attached but may not be opened: offer the permission prompt. */
    data class NeedsPermission(override val text: String) : ConnectionStrip

    data class Info(override val text: String) : ConnectionStrip

    /** Demo mode is on: always visible, always labelled, with the way out. */
    data object Demo : ConnectionStrip {
        override val text = "Demo (no device): a simulated DSPi. Nothing is sent over USB."
    }

    /**
     * REMOTE (DSPi Link): always visible, so a remote unit is never mistaken for the USB one.
     * [lost] (WP AN5): the connection is lost and the core is reconnecting, shown as the
     * owner's banner; [detail], the banner's second line, is the hub and the core's `close_reason`.
     */
    data class Remote(override val text: String, val actions: List<RemoteAction>, val lost: Boolean = false, val detail: String = "") :
        ConnectionStrip
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

fun connectionStrip(
    mode: AppMode,
    state: UsbSessionState,
    demoAvailable: Boolean,
    remote: LinkSessionState = LinkSessionState.Idle,
    networkAvailable: Boolean = false,
    lockNotice: Boolean = false,
    wording: LinkWording = LinkWording.CORE
): ConnectionStrip = when {
    mode == AppMode.DEMO -> ConnectionStrip.Demo
    mode == AppMode.REMOTE -> remoteStrip(remote, lockNotice, wording)
    else -> when (state) {
        UsbSessionState.NoDevice -> ConnectionStrip.NoDevice(demoAvailable, networkAvailable)
        is UsbSessionState.PermissionRequired ->
            ConnectionStrip.NeedsPermission("A DSPi is connected. Allow USB access to use it. $USB_ALWAYS_OPEN_HINT")
        is UsbSessionState.PermissionDenied -> ConnectionStrip.NeedsPermission("USB access to the DSPi was denied.")
        // queued prompts are shown by the session as soon as a UI is visible: no second request
        is UsbSessionState.PermissionPending -> ConnectionStrip.Info("Waiting for USB permission… $USB_ALWAYS_OPEN_HINT")
        is UsbSessionState.Failed -> ConnectionStrip.Info("Could not open the DSPi: ${state.reason}")
        is UsbSessionState.Open -> ConnectionStrip.Hidden
    }
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
