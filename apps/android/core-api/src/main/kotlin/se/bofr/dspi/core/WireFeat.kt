package se.bofr.dspi.core

/**
 * WP AN6: the bits of [DspiCoreApi.wireFeatures] (`dspi_proto::wire_compat::feat`;
 * `WireFeatTest` compares this table with the Rust source).
 */
object WireFeat {
    const val ADAT_OUT = 1L shl 0
    const val LEVELLER_MASKS = 1L shl 1
    const val LOUDNESS_MASK = 1L shl 2
    const val CROSSFEED_MASK = 1L shl 3

    /** I2S clock master/slave, S/PDIF inputs 2-3, I2S clock-pin mode. */
    const val I2S_CLOCK_MODE = 1L shl 4
    const val LINKWITZ = 1L shl 5
    const val PSYBASS = 1L shl 6
    const val ADAT_IN = 1L shl 7
    const val UPMIX = 1L shl 8
    const val UPMIX_PRESENCE = 1L shl 9

    /** Fourth S/PDIF input; first-order low/high pass filter types. */
    const val SPDIF_IN4 = 1L shl 10
    const val SUBHARM = 1L shl 11
    const val SUBHARM_EXT = 1L shl 12
    const val TUBE = 1L shl 13
    const val LIMITER = 1L shl 14

    /** Every feature the V32 layout carries. */
    const val ALL = (1L shl 15) - 1

    /** (feature, first wire format that carries it). */
    val SINCE: List<Pair<Long, Int>> = listOf(
        ADAT_OUT to 17,
        LEVELLER_MASKS to 18,
        LOUDNESS_MASK to 19,
        CROSSFEED_MASK to 20,
        I2S_CLOCK_MODE to 21,
        LINKWITZ to 22,
        PSYBASS to 23,
        ADAT_IN to 24,
        UPMIX to 25,
        UPMIX_PRESENCE to 26,
        SPDIF_IN4 to 28,
        SUBHARM to 29,
        SUBHARM_EXT to 30,
        TUBE to 31,
        LIMITER to 32
    )

    /** `wire_compat::features`, for test doubles: a real client asks the core. */
    fun of(version: Int): Long = if (version < DspiConstants.MIN_WIRE_FORMAT_VERSION) {
        0L
    } else {
        SINCE.filter { version >= it.second }.fold(0L) { m, f -> m or f.first }
    }
}
