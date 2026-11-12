package se.bofr.dspi.feature.home

import org.junit.Assert.assertEquals
import org.junit.Test
import se.bofr.dspi.feature.home.HomeModel.Health
import se.bofr.dspi.feature.home.HomeModel.HealthLine

/** WP UIM1: the words and state of the phone Home's health line. */
class HealthLineWpUim1Test {
    private fun line(
        connected: Boolean = true,
        clipping: List<String> = emptyList(),
        limiting: List<String> = emptyList(),
        cpu0: Int = 20,
        cpu1: Int = 10
    ) = HomeModel.healthLine(connected, clipping, limiting, cpu0, cpu1)

    @Test
    fun uim1NoDeviceIsOfflineWhateverElseIsSet() {
        assertEquals(HealthLine("Not connected", Health.OFFLINE), line(connected = false, clipping = listOf("OUT1"), cpu0 = 99))
    }

    @Test
    fun uim1QuietDeviceIsOk() {
        assertEquals(HealthLine("All outputs OK", Health.OK), line())
        // At the limit is still fine: the CPU meter turns red only above it
        assertEquals(Health.OK, line(cpu0 = HomeModel.CPU_HIGH, cpu1 = HomeModel.CPU_HIGH).health)
    }

    @Test
    fun uim1ClippingIsAProblemAndNamesTheOutputs() {
        assertEquals(HealthLine("Clipping: OUT1, OUT3", Health.PROBLEM), line(clipping = listOf("OUT1", "OUT3")))
    }

    @Test
    fun uim1LimitingAndLoadAreWarnings() {
        assertEquals(HealthLine("Limiting: OUT2", Health.WARNING), line(limiting = listOf("OUT2")))
        assertEquals(HealthLine("CPU load high", Health.WARNING), line(cpu1 = 95))
    }

    @Test
    fun uim1EverythingAtOnceIsOneLineWorstStateFirst() {
        assertEquals(
            HealthLine("Clipping: OUT1 · Limiting: OUT2 · CPU load high", Health.PROBLEM),
            line(clipping = listOf("OUT1"), limiting = listOf("OUT2"), cpu0 = 91)
        )
    }
}
