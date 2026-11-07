// WP AN35: AN18's HostAudioFormat wired into the strip layout (M25), as
// macOS HostAudioFormatMonitor feeds DSPViewModel.effectiveInputChannels
// (DSPViewModel.swift:1947-1951). The count is read again whenever Android's
// outputs change (AudioDeviceCallback), so the strips follow the host while
// the device's own report idles to stereo. The pair links follow the same
// count: a pair the strips show can be linked.
package se.bofr.dspi.feature.home

import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.platform.LocalContext
import se.bofr.dspi.bridge.DspiBridge

/** Where the host channel count comes from: `AudioManager` in the app, a fake in tests. */
interface HostChannelSource {
    /** [HostAudioFormat.channelCount] now. */
    fun channelCount(): Int?

    /** Calls [onChange] (main thread) whenever the host's outputs change, until closed. */
    fun watch(onChange: () -> Unit): AutoCloseable
}

/** [HostChannelSource] over `AudioManager`: [HostAudioFormatReader], refreshed by an `AudioDeviceCallback`. */
class AudioManagerHostChannelSource(context: Context) : HostChannelSource {
    private val audio = context.applicationContext.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
    private val reader = HostAudioFormatReader(context)

    override fun channelCount(): Int? = reader.channelCount()

    override fun watch(onChange: () -> Unit): AutoCloseable {
        val am = audio ?: return AutoCloseable {}
        val callback =
            object : AudioDeviceCallback() {
                override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = onChange()

                override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = onChange()
            }
        am.registerAudioDeviceCallback(callback, Handler(Looper.getMainLooper()))
        return AutoCloseable { am.unregisterAudioDeviceCallback(callback) }
    }
}

/** The host count source; null: the screen's own [AudioManagerHostChannelSource]. */
val LocalHostChannelSource = staticCompositionLocalOf<HostChannelSource?> { null }

/** The host channel count, kept current while composed. */
@Composable
internal fun hostChannelCount(): Int? {
    val source = LocalHostChannelSource.current ?: LocalContext.current.let { ctx -> remember(ctx) { AudioManagerHostChannelSource(ctx) } }
    var count by remember(source) { mutableStateOf(source.channelCount()) }
    DisposableEffect(source) {
        val watch = source.watch { count = source.channelCount() }
        count = source.channelCount()
        onDispose { watch.close() }
    }
    return count
}

/**
 * How many input strips to lay out: [HostAudioFormat.effectiveInputChannels]
 * over the device's report and the host count. 0 until the device has
 * reported (no inputs, or an unknown active count).
 */
@Composable
internal fun stripInputCount(bridge: DspiBridge): Int {
    val s = bridge.observeState()
    val active = bridge.observeStatus { it.activeInputChannels }
    val host = hostChannelCount()
    return HostStrips.count(host, active, s.inputSource, s.numInputChannels, s.i2sInputChannels)
}

internal object HostStrips {
    /** [stripInputCount]'s rule. */
    fun count(host: Int?, active: Int, inputSource: Int, numInputChannels: Int, i2sInputChannels: Int): Int =
        if (active < 0 || numInputChannels <= 0) {
            0
        } else {
            HostAudioFormat.effectiveInputChannels(host, active, inputSource, numInputChannels, i2sInputChannels)
        }

    /** Input [input]'s pair is laid out whole, so it can be linked (the bridge's `pairAvailable` over [strips]). */
    fun pairAvailable(input: Int, strips: Int): Boolean = input >= 0 && (input or 1) < strips

    /** Input [input]'s pair is linked in [links] (one bit per pair) and laid out whole. */
    fun linked(input: Int, strips: Int, links: Int): Boolean = pairAvailable(input, strips) && (links shr (input / 2)) and 1 == 1
}
