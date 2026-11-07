package se.bofr.dspi.feature.home

import android.media.AudioDeviceInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_ADAT
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_I2S
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_SPDIF3
import se.bofr.dspi.feature.home.HostAudioFormat.SOURCE_USB

/** WP AN18: the host format from Android's audio outputs, and the strip count it gives. */
class HostAudioFormatWpAn18Test {
    private val speaker = HostOutput(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, "Phone", listOf(2))

    @Test
    fun wpAn18TheDspiUsbOutputGivesTheCount() {
        assertEquals(
            8,
            HostAudioFormat.channelCount(listOf(speaker, HostOutput(AudioDeviceInfo.TYPE_USB_DEVICE, "Weeb Labs DSPi", listOf(2, 4, 6, 8))))
        )
        assertEquals(4, HostAudioFormat.channelCount(listOf(HostOutput(AudioDeviceInfo.TYPE_USB_HEADSET, "dspi", listOf(1, 2, 3, 4)))))
    }

    @Test
    fun wpAn18NoDspiOrNoFixedCountFallsBack() {
        assertNull(HostAudioFormat.channelCount(listOf(speaker)))
        assertNull(HostAudioFormat.channelCount(listOf(HostOutput(AudioDeviceInfo.TYPE_USB_DEVICE, "Other DAC", listOf(8)))))
        assertNull(HostAudioFormat.channelCount(listOf(HostOutput(AudioDeviceInfo.TYPE_USB_DEVICE, "Weeb Labs DSPi", emptyList()))))
        assertNull(HostAudioFormat.channelCount(emptyList()))
    }

    @Test
    fun wpAn18EffectiveInputsFollowTheMac() {
        // USB: the host format survives the device idling to its stereo fallback
        assertEquals(8, HostAudioFormat.effectiveInputChannels(8, 2, SOURCE_USB, 8, 2))
        assertEquals(2, HostAudioFormat.effectiveInputChannels(null, 2, SOURCE_USB, 8, 2))
        assertEquals(6, HostAudioFormat.effectiveInputChannels(null, 6, SOURCE_USB, 8, 2))
        // never above what the device has (RP2040: 2)
        assertEquals(2, HostAudioFormat.effectiveInputChannels(8, 2, SOURCE_USB, 2, 2))
        // other sources ignore the host
        assertEquals(2, HostAudioFormat.effectiveInputChannels(8, 8, SOURCE_SPDIF3, 8, 2))
        assertEquals(4, HostAudioFormat.effectiveInputChannels(8, 2, SOURCE_I2S, 8, 4))
        assertEquals(8, HostAudioFormat.effectiveInputChannels(null, 2, SOURCE_ADAT, 8, 2))
        assertEquals(2, HostAudioFormat.effectiveInputChannels(null, 0, SOURCE_USB, 8, 2))
    }
}
