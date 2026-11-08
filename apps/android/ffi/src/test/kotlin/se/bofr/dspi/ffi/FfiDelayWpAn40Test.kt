package se.bofr.dspi.ffi

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WP AN40: fractional delays on the real core (the demo device, through JNA).
 * A configuration document with 0.5 and 1.5 ms output delays applies exactly,
 * and a fractional delay read back from the device survives unrelated writes.
 */
class FfiDelayWpAn40Test {
    private fun demo() = FfiDspiCore().apply { openDemo() }

    /** The document [doc] with its first output delays replaced by [values], in channel order. */
    private fun withOutputDelays(doc: String, vararg values: Double): String {
        var n = 0
        val out =
            Regex(""""outputDelayMs"\s*:\s*-?[0-9.eE+-]+""").replace(doc) { m ->
                if (n < values.size) "\"outputDelayMs\":${values[n++]}" else m.value
            }
        assertEquals("output delays replaced", values.size, n)
        return out
    }

    @Test
    fun wpAn40HalfMillisecondPresetAppliesExactly() {
        demo().use { core ->
            val doc = withOutputDelays(core.configExport("AN40", "test", 0), 0.5, 1.5)
            val r = core.configImport(doc, 0)
            assertTrue(r, r.contains("\"ok\":true"))
            assertEquals(0.5f, core.getState().outputDelayMs[0], 0f)
            assertEquals(1.5f, core.getState().outputDelayMs[1], 0f)
            // what the device holds now, read back
            core.refreshParams()
            assertEquals(0.5f, core.getState().outputDelayMs[0], 0f)
            assertEquals(1.5f, core.getState().outputDelayMs[1], 0f)
        }
    }

    @Test
    fun wpAn40FractionalDelayFromTheDeviceSurvivesAnUnrelatedWrite() {
        demo().use { core ->
            core.setOutputDelay(0, 0.25f)
            core.setOutputDelay(1, 1.5f)
            core.refreshParams()
            assertEquals(0.25f, core.getState().outputDelayMs[0], 0f)

            core.setOutputGain(0, -3f)
            core.setOutputMute(0, true)
            core.setOutputDelay(2, 3f)
            core.setPreamp(-2f)
            core.refreshParams()
            val s = core.getState()
            assertEquals(0.25f, s.outputDelayMs[0], 0f)
            assertEquals(1.5f, s.outputDelayMs[1], 0f)
            assertEquals(3f, s.outputDelayMs[2], 0f)
        }
    }
}
