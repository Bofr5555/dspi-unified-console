package se.bofr.dspi.feature.home

import org.junit.Assert.assertEquals
import org.junit.Test

/** WP P1AN2 (S8): "stale since" on Home's health line when the core says the meters are old. */
class HealthLineStaleWpP1an2Test {
    @Test
    fun wpP1an2StaleMetersComeFirstAndAreNeverOk() {
        val fresh = HomeModel.healthLine(true, emptyList(), emptyList(), 10, 10)
        assertEquals(HomeModel.Health.OK, fresh.health)
        val stale = HomeModel.healthLine(true, emptyList(), emptyList(), 10, 10, staleMs = 12_400)
        assertEquals("Meters stale since 12 s", stale.text)
        assertEquals(HomeModel.Health.WARNING, stale.health)
        assertEquals(HomeModel.Health.PROBLEM, HomeModel.healthLine(true, listOf("OUT1"), emptyList(), 0, 0, staleMs = 300).health)
        assertEquals("Meters stale since 1 s", HomeModel.staleLine(300))
        assertEquals(HomeModel.Health.OFFLINE, HomeModel.healthLine(false, emptyList(), emptyList(), 0, 0, staleMs = 5_000).health)
    }
}
