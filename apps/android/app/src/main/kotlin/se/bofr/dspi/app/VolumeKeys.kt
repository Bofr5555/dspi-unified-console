// WP ANVOL: the hardware volume buttons move the DSPi's User Volume (`vol.user`, the Home
// "User Volume" slider) while the app is in front and a DSPi is the current device (USB,
// DSPi Link or the demo). Otherwise the keys go on unhandled and Android does what it always
// does. Only the Activity's own key dispatch is used: no media session, no accessibility
// service, so nothing is taken from Android while the app is in the background.
package se.bofr.dspi.app

import android.view.KeyEvent
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.roundToInt
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge

/** What the volume buttons act on; the Activity's view of the current bridge and its settings. */
internal interface VolumeKeyHost {
    /** The setting "Volume Buttons Control User Volume" (default on). */
    val enabled: Boolean

    /** The Activity is resumed (in front). */
    val foreground: Boolean

    /** A DSPi is the current device (USB, DSPi Link or demo) and the connection takes writes. */
    val deviceWritable: Boolean

    /** The device's User Volume, dB, as the bridge last knew it. */
    val userVolumeDb: Float

    val userMute: Boolean

    fun setUserVolume(db: Float)

    fun setUserMute(muted: Boolean)
}

/**
 * [VolumeKeyHost] on the current bridge: a DSPi is the current device while the bridge is
 * connected (USB, DSPi Link or the demo), takes writes (not read-only, not a DSPi Link viewer)
 * and is no document opened offline (P1AN1: no DSPi behind it).
 */
internal class BridgeVolumeKeyHost(
    private val bridge: () -> DspiBridge,
    private val setting: () -> Boolean,
    private val resumed: () -> Boolean
) : VolumeKeyHost {
    override val enabled get() = setting()
    override val foreground get() = resumed()
    override val deviceWritable get() = bridge().let { b ->
        b.connected && b.writesAllowed.value && b.snapshots.offline.value == null
    }
    override val userVolumeDb get() = bridge().userVolumeDB
    override val userMute get() = bridge().userMute

    override fun setUserVolume(db: Float) = bridge().setUserVolume(db)

    override fun setUserMute(muted: Boolean) = bridge().setUserMute(muted)
}

/** The level a button press set, for the on-screen indicator; [seq] tells two equal levels apart. */
internal data class VolumeKeyFeedback(val db: Float, val muted: Boolean, val seq: Int)

/**
 * The volume buttons' rules:
 * - one press (or one key repeat while held) moves the User Volume 1 dB, on the whole-dB grid
 *   the firmware applies (its volume index is whole dB, UAC1 resolution 1 dB): from -23.4 up
 *   is -23, down is -24; held to -60..0 dB, the range the core and the firmware clamp to;
 * - writes are paced: the first press is sent at once, then at most one write per
 *   [SEND_INTERVAL_MS] carrying the latest level, so a held key sends at most 10 writes a
 *   second over USB or DSPi Link and the last level always goes out;
 * - the next press counts from the level last asked for while a write may still be on its way
 *   ([HOLD_MS]), so a slow echo never makes a press step back;
 * - KEYCODE_VOLUME_MUTE toggles the User Volume's mute (`vol.mute`), as Command+M does;
 * - inactive (setting off, not in front, no DSPi or a read-only connection) a key is not
 *   handled, and its key-up is left alone too; a key-up whose key-down was handled is handled.
 *
 * Main thread only. [now] is the clock in ms ([android.os.SystemClock.uptimeMillis]).
 */
internal class VolumeKeyController(private val scope: CoroutineScope, private val host: VolumeKeyHost, private val now: () -> Long) {
    private val feedbackState = MutableStateFlow<VolumeKeyFeedback?>(null)

    /** The last level a press set; null until the first press. */
    val feedback: StateFlow<VolumeKeyFeedback?> = feedbackState.asStateFlow()

    /** The level last asked for, and when. */
    private var target: Float? = null
    private var targetAt = 0L

    /** The level last written; [pacer] writes [target] when it differs. */
    private var sent: Float? = null
    private var pacer: Job? = null

    /** Key codes whose key-down was handled, so their key-up is too. */
    private val down = mutableSetOf<Int>()

    /** [KeyEvent] in: true when the key was handled (Android must not act on it). */
    fun onKey(event: KeyEvent): Boolean = onKey(event.keyCode, event.action, event.repeatCount)

    fun onKey(keyCode: Int, action: Int, repeatCount: Int): Boolean {
        if (keyCode !in KEYS) return false
        if (action == KeyEvent.ACTION_UP) return down.remove(keyCode)
        if (action != KeyEvent.ACTION_DOWN) return false
        if (!active()) {
            down.remove(keyCode)
            return false
        }
        down += keyCode
        when (keyCode) {
            KeyEvent.KEYCODE_VOLUME_UP -> step(+1)
            KeyEvent.KEYCODE_VOLUME_DOWN -> step(-1)
            // a held mute key repeats; one toggle per press
            KeyEvent.KEYCODE_VOLUME_MUTE -> if (repeatCount == 0) toggleMute()
        }
        return true
    }

    private fun active(): Boolean = host.enabled && host.foreground && host.deviceWritable

    private fun step(direction: Int) {
        val t = target
        val base = if (t != null && now() - targetAt < HOLD_MS) t else host.userVolumeDb
        val next = nextLevel(base, direction)
        target = next
        targetAt = now()
        show(next, host.userMute)
        if (pacer?.isActive != true) {
            // at an end of the range a press changes nothing: shown, not sent
            if (next != base) write(next) else sent = next
            pacer = scope.launch {
                while (true) {
                    delay(SEND_INTERVAL_MS)
                    val latest = target ?: break
                    // the device went away or turned read-only meanwhile: nothing more to send
                    if (latest == sent || !host.deviceWritable) break
                    write(latest)
                }
            }
        }
    }

    private fun write(db: Float) {
        sent = db
        host.setUserVolume(db)
    }

    private fun toggleMute() {
        val muted = !host.userMute
        host.setUserMute(muted)
        show(target?.takeIf { now() - targetAt < HOLD_MS } ?: host.userVolumeDb, muted)
    }

    private fun show(db: Float, muted: Boolean) {
        feedbackState.value = VolumeKeyFeedback(db, muted, (feedbackState.value?.seq ?: 0) + 1)
    }

    companion object {
        /** The keys this handles. */
        val KEYS = setOf(KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_VOLUME_DOWN, KeyEvent.KEYCODE_VOLUME_MUTE)

        const val MIN_DB = -60f
        const val MAX_DB = 0f

        /** At most one write per this many ms while a key is held (10 a second). */
        const val SEND_INTERVAL_MS = 100L

        /** A press counts from the level last asked for while it is this recent. */
        const val HOLD_MS = 1_000L

        /**
         * One step from [db] in [direction] (+1 up, -1 down) on the whole-dB grid, held to
         * [MIN_DB]..[MAX_DB]. [db] is first rounded to 0.01 dB, so the firmware's 8.8
         * fixed-point echo (-23.0039) counts as -23.
         */
        fun nextLevel(db: Float, direction: Int): Float {
            val d = if (db.isFinite()) (db * 100f).roundToInt() / 100f else MAX_DB
            val next = if (direction > 0) floor(d) + 1f else ceil(d) - 1f
            return next.coerceIn(MIN_DB, MAX_DB)
        }
    }
}
