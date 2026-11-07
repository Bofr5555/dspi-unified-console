// WP AN18: the host format, Android's side of macOS HostAudioFormatMonitor.swift
// (and SW20's iOS route count). The firmware reports its ACTIVE input count
// from the USB alternate setting the host has open, which drops to a stereo
// fallback while nothing plays. When Android lists the DSPi as a USB output,
// its channel counts say how many channels the host can send, and stand in
// for the device's count the way CoreAudio's selected format does on the Mac;
// with no such device the device's report is used, as on iOS.
// `effectiveInputChannels` is DSPViewModel.effectiveInputChannels.
package se.bofr.dspi.feature.home

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager

/** One output Android lists (`AudioDeviceInfo`), reduced to what the reader looks at. */
data class HostOutput(val type: Int, val productName: String, val channelCounts: List<Int>)

object HostAudioFormat {
    /** `BASE_MATRIX_INPUTS`. */
    const val BASE_INPUTS = 2

    /** The widest USB format a DSPi takes. */
    const val MAX_INPUTS = 8

    // macOS Constants.swift INPUT_SOURCE_*
    const val SOURCE_USB = 0
    const val SOURCE_SPDIF = 1
    const val SOURCE_I2S = 2
    const val SOURCE_ADAT = 3
    const val SOURCE_SPDIF2 = 4
    const val SOURCE_SPDIF3 = 5
    const val SOURCE_SPDIF4 = 6

    /** USB product string "Weeb Labs DSPi" (macOS `matchToken`). */
    private const val MATCH_TOKEN = "DSPi"

    /**
     * The DSPi's host channel count (2..8, even), or null when Android lists
     * no DSPi output or names no fixed counts (an empty list means "any").
     */
    fun channelCount(outputs: List<HostOutput>): Int? {
        val dspi = outputs.firstOrNull {
            (it.type == AudioDeviceInfo.TYPE_USB_DEVICE || it.type == AudioDeviceInfo.TYPE_USB_HEADSET) &&
                it.productName.contains(MATCH_TOKEN, ignoreCase = true)
        } ?: return null
        val best = dspi.channelCounts.filter { it in BASE_INPUTS..MAX_INPUTS && it % 2 == 0 }.maxOrNull()
        return best
    }

    /**
     * `effectiveInputChannels`: how many input strips to lay out. USB follows
     * the host format ([hostCount]) and falls back to the device's
     * [activeInputChannels]; S/PDIF is stereo, I2S its configured count, ADAT 8.
     * Never below 2 or above the device's [numInputChannels].
     */
    fun effectiveInputChannels(
        hostCount: Int?,
        activeInputChannels: Int,
        inputSource: Int,
        numInputChannels: Int,
        i2sInputChannels: Int
    ): Int {
        val base = when (inputSource) {
            SOURCE_SPDIF, SOURCE_SPDIF2, SOURCE_SPDIF3, SOURCE_SPDIF4 -> BASE_INPUTS
            SOURCE_I2S -> i2sInputChannels
            SOURCE_ADAT -> MAX_INPUTS
            else -> hostCount ?: activeInputChannels
        }
        return maxOf(BASE_INPUTS, minOf(base, maxOf(BASE_INPUTS, numInputChannels)))
    }
}

/** Reads the host format from `AudioManager`. Cheap, no I/O; call when the outputs change. */
class HostAudioFormatReader(context: Context) {
    private val audio = context.applicationContext.getSystemService(Context.AUDIO_SERVICE) as? AudioManager

    /** [HostAudioFormat.channelCount] of what Android lists now; null without an audio service. */
    fun channelCount(): Int? {
        val devices = audio?.getDevices(AudioManager.GET_DEVICES_OUTPUTS) ?: return null
        return HostAudioFormat.channelCount(
            devices.map { HostOutput(it.type, it.productName?.toString().orEmpty(), it.channelCounts.toList()) }
        )
    }
}
