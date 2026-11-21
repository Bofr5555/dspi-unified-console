package se.bofr.dspi.link

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WP ANSEC (owner 2026-11-21): the acknowledgement that a hub's connection is unsecured is kept
 * per hub (its id, and for a typed hub its address) across restarts, dropped when the hub's
 * security or identity changes, and forgotten per hub or all at once.
 */
class UnsecuredAcksWpAnsecTest {
    private val kv = InMemoryKeyValue()
    private val plain = HubSecurity("6f1a", encrypted = false, hubKey = "")
    private val studio = LinkHub("6f1a", "Studio Mac", "console", "pin", 1, emptyList(), "192.168.1.20", 11915)
    private val typed = LinkHub.manual("192.168.1.30", 11915).copy(id = "9c2e", name = "Bridge")

    @Test
    fun wpAnsecAnAcknowledgementIsRememberedAcrossRestarts() {
        val store = UnsecuredAckStore(kv)
        assertFalse(store.acknowledged(studio))
        store.acknowledge(studio)
        assertTrue(store.acknowledged(studio))
        assertTrue("by id, any case", store.acknowledged(studio.copy(id = "6F1A")))
        assertFalse("another hub", store.acknowledged(studio.copy(id = "7b00")))
        // A restart: a new store over the same storage
        val again = UnsecuredAckStore(kv)
        assertTrue(again.acknowledged(studio))
        assertEquals(listOf(UnsecuredAck("6f1a", "Studio Mac")), again.acks.value)
        // Still unsecured: nothing changes
        assertFalse(again.observe(studio, plain))
        assertTrue(again.acknowledged(studio))
    }

    @Test
    fun wpAnsecAnEncryptedConnectionDropsIt() {
        val store = UnsecuredAckStore(kv)
        store.acknowledge(studio)
        // Secured (pinned) now: a later unsecured connection is a change and is highlighted again
        assertTrue(store.observe(studio, plain.copy(encrypted = true, hubKey = "SHA256:K7Q2-M9XD-4TNB-0ZRA")))
        assertFalse(store.acknowledged(studio))
        assertFalse(store.observe(studio, plain))
        assertFalse(store.acknowledged(studio))
        assertFalse("gone from storage too", UnsecuredAckStore(kv).acknowledged(studio))
        // Another hub's encrypted connection leaves it alone
        store.acknowledge(studio)
        assertFalse(store.observe(studio.copy(id = "7b00"), plain.copy(hubId = "7b00", encrypted = true)))
        assertTrue(store.acknowledged(studio))
    }

    @Test
    fun wpAnsecATypedHubKeepsItsAddressAndAnotherHubThereDropsIt() {
        val store = UnsecuredAckStore(kv)
        store.acknowledge(typed)
        assertEquals(UnsecuredAck("9c2e", "Bridge", "192.168.1.30", 11915), store.acks.value.single())
        assertTrue(store.acknowledged(typed))
        assertFalse("typed at another address", store.acknowledged(typed.copy(host = "192.168.1.31")))
        assertTrue("the same hub found by DNS-SD", store.acknowledged(typed.copy(isManual = false, host = "bridge.local")))
        // A different hub answers at that address
        val other = LinkHub.manual("192.168.1.30", 11915).copy(id = "d00d", name = "Other")
        assertTrue(store.observe(other, plain.copy(hubId = "d00d")))
        assertFalse(store.acknowledged(typed))
        assertTrue(store.acks.value.isEmpty())
    }

    @Test
    fun wpAnsecForgetOneOrAll() {
        val store = UnsecuredAckStore(kv)
        store.acknowledge(studio)
        store.acknowledge(typed)
        assertEquals(listOf("Bridge", "Studio Mac"), store.acks.value.map { it.name })
        store.forget("6F1A")
        assertFalse(store.acknowledged(studio))
        assertTrue(store.acknowledged(typed))
        store.forgetAll()
        assertTrue(store.acks.value.isEmpty())
        assertTrue(UnsecuredAckStore(kv).acks.value.isEmpty())
        // Unreadable rows are skipped
        kv.put("ack:bad", "v0")
        assertTrue(UnsecuredAckStore(kv).acks.value.isEmpty())
    }
}
