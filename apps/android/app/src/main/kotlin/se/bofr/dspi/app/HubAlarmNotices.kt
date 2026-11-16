// WP P2AN: the hub's alarms as notices over every screen (PLAN-P2 M3 for a
// single-device console), and the strip's Identify. An alarm the hub raises
// for the open device (or for no one device: the audio link, the hub) is
// shown once, in the core's notice row (`hub_alarm_notice`, CG10's shared
// words), with Acknowledge where this console may change things; the hub
// decides who may (Control or Admin). Alarms queue: one alert at a time, as
// the AlertHost's one slot. Nothing shows on a hub without the `alarms` cap.
package se.bofr.dspi.app

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.HubServer
import se.bofr.dspi.core.HubAlarm
import se.bofr.dspi.core.Notice
import se.bofr.dspi.ui.alert.AlertButton
import se.bofr.dspi.ui.alert.AlertButtonRole
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertStyle
import se.bofr.dspi.ui.alert.ConsoleAlert
import se.bofr.dspi.ui.onMainThread

class HubAlarmNotices(private val bridge: DspiBridge, private val scope: CoroutineScope, private val noticeLine: (String) -> Unit = {}) {
    /** The alarms' own alert slot, drawn by the scaffold. */
    val host = AlertHost()

    private val queue = ArrayDeque<HubAlarm>()
    private var showing = false

    /** Show each raised alarm until the caller's scope ends. Main thread. */
    suspend fun run() {
        bridge.hubServer.raised.collect { offer(it) }
    }

    /** Queue [a]; shown at once when nothing else is. Main thread. */
    fun offer(a: HubAlarm) {
        if (queue.any { it.id == a.id }) return
        queue.addLast(a)
        if (!showing) next()
    }

    private fun next() {
        val a = queue.removeFirstOrNull()
        if (a == null) {
            showing = false
            return
        }
        showing = true
        scope.launch {
            // Still on? Another console may have acknowledged it meanwhile
            if (bridge.hubServer.alarms.value.none { it.id == a.id && it.state == HubServer.ALARM_RAISED }) return@launch next()
            val notice = bridge.hubServer.alarmNotice(a)
            // -- WP ANDFIX1 -- back on main after the FFI call: the alert host's state and the queue are main-thread only
            onMainThread {
                val alert = alertOf(a, notice, bridge.writesAllowed.value, onAcknowledge = { acknowledge(a) }, onLater = ::next)
                host.show(alert)
            }
        }
    }

    private fun acknowledge(a: HubAlarm) {
        scope.launch {
            val r = bridge.hubServer.acknowledge(a.id)
            // -- WP ANDFIX1 -- the notice line and the queue on main
            onMainThread {
                r.onFailure { noticeLine(it.message ?: "The alarm could not be acknowledged.") }
                next()
            }
        }
    }

    companion object {
        const val ACKNOWLEDGE = "Acknowledge"
        const val LATER = "Later"

        /** The alert for [a]: the core's row (title, reason, action, the hub's words), else the hub's own line. */
        fun alertOf(a: HubAlarm, notice: Notice?, canAcknowledge: Boolean, onAcknowledge: () -> Unit, onLater: () -> Unit): ConsoleAlert {
            val title = notice?.title?.takeIf { it.isNotBlank() } ?: "Alarm"
            val body = listOfNotNull(notice?.reason, notice?.action, notice?.detail ?: a.message)
                .map { it.trim() }.filter { it.isNotEmpty() }.distinct().joinToString("\n\n")
            val buttons = listOfNotNull(
                AlertButton(ACKNOWLEDGE, AlertButtonRole.DEFAULT) { onAcknowledge() }.takeIf { canAcknowledge },
                AlertButton(LATER, if (canAcknowledge) AlertButtonRole.CANCEL else AlertButtonRole.DEFAULT) { onLater() }
            )
            return ConsoleAlert(title, body, if (a.severity == "critical") AlertStyle.CRITICAL else AlertStyle.WARNING, buttons)
        }
    }
}

/** The strip's Identify: the open device's LED blinks (through the hub where it has the cap); the outcome on the notice line. */
class IdentifyAction(private val bridge: DspiBridge, private val scope: CoroutineScope, private val noticeLine: (String) -> Unit) {
    fun run() {
        scope.launch {
            val r = bridge.hubServer.identify(IDENTIFY_SECONDS)
            // -- WP ANDFIX1 -- the call resumes on the FFI thread under a test dispatcher: the notice line (Compose state) on main
            onMainThread {
                r.onSuccess { noticeLine(BLINKING) }
                    .onFailure { noticeLine(it.message ?: "Identify did not work.") }
            }
        }
    }

    companion object {
        const val IDENTIFY_SECONDS = 10
        const val BLINKING = "The DSPi's LED blinks for 10 seconds."
    }
}

/** The strip's Identify entry. */
internal const val IDENTIFY_ENTRY = "Identify"
