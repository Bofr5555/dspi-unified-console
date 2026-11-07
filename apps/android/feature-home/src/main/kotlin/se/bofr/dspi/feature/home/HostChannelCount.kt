// WP AN35: AN18's HostAudioFormat wired into the strip layout (M25), as
// macOS HostAudioFormatMonitor feeds DSPViewModel.effectiveInputChannels
// (DSPViewModel.swift:1947-1951). The count is read again whenever Android's
// outputs change (AudioDeviceCallback) and handed to the bridge, whose
// liveInputCount (macOS numMatrixInputs) is then the one count for the
// strips and for the pair links it mirrors, so the strips follow the host
// while the device's own report idles to stereo.
package se.bofr.dspi.feature.home

import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
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

/** The host count source; null: an [AudioManagerHostChannelSource] over the context. */
val LocalHostChannelSource = staticCompositionLocalOf<HostChannelSource?> { null }

/**
 * Hands the host count to [bridge] while composed (the app calls it once, over
 * every route). [enabled] false (demo, DSPi Link: the DSPi is not on this
 * phone's USB) gives the bridge none.
 */
@Composable
fun HostChannelsFeed(bridge: DspiBridge, enabled: Boolean = true) {
    val source = LocalHostChannelSource.current ?: LocalContext.current.let { ctx -> remember(ctx) { AudioManagerHostChannelSource(ctx) } }
    DisposableEffect(bridge, source, enabled) {
        if (!enabled) {
            bridge.setHostInputChannels(null)
            return@DisposableEffect onDispose {}
        }
        val watch = source.watch { bridge.setHostInputChannels(source.channelCount()) }
        bridge.setHostInputChannels(source.channelCount())
        onDispose {
            watch.close()
            bridge.setHostInputChannels(null)
        }
    }
}

/**
 * How many input strips to lay out: the bridge's [DspiBridge.liveInputCount]
 * (the host's format on USB). 0 until the device has reported its active count.
 */
@Composable
internal fun stripInputCount(bridge: DspiBridge): Int {
    bridge.observeState() // a host count change bumps the revision
    val active = bridge.observeStatus { it.activeInputChannels }
    return if (active < 0) 0 else bridge.liveInputCount()
}
