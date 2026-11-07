package se.bofr.dspi.core

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Test

/** WP AN6: the Kotlin table cannot drift from `dspi_proto::wire_compat::feat`. */
class WireFeatTest {
    private val source = File("../../../crates/dspi-proto/src/wire_compat.rs").readText()

    @Test
    fun wpAn6BitsMatchTheRustTable() {
        val bits = Regex("""pub const (\w+): u32 = 1 << (\d+);""").findAll(source)
            .associate { it.groupValues[1] to (1L shl it.groupValues[2].toInt()) }
        val kotlin = WireFeat::class.java.fields.filter { it.type == Long::class.javaPrimitiveType && it.name != "ALL" }
            .associate { it.name to it.getLong(null) }
        assertEquals(bits, kotlin)
    }

    @Test
    fun wpAn6VersionsMatchTheRustTable() {
        val since = source.substringAfter("SINCE:").substringBefore("];")
        val rust = Regex("""\((\w+), (\d+)\)""").findAll(since)
            .associate { it.groupValues[1] to it.groupValues[2].toInt() }
        val byBit = WireFeat::class.java.fields.filter { it.type == Long::class.javaPrimitiveType }
            .associate { it.getLong(null) to it.name }
        assertEquals(rust, WireFeat.SINCE.associate { byBit.getValue(it.first) to it.second })
    }

    @Test
    fun wpAn6FeaturesOfAVersion() {
        assertEquals(0L, WireFeat.of(15))
        assertEquals(0L, WireFeat.of(16))
        assertEquals(WireFeat.ADAT_OUT, WireFeat.of(17))
        assertEquals(WireFeat.ALL, WireFeat.of(32))
        assertEquals(WireFeat.ALL, WireFeat.of(40))
    }
}
