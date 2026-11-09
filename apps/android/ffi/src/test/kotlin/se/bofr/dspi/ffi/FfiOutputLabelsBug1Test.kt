package se.bofr.dspi.ffi

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.OutputCard

/**
 * BUG1: the core's output labels and dashboard cards through the adapter, on the demo device
 * with the K340's own configuration applied (tests/fixtures/preset/k340-bug1.dspipreset).
 */
class FfiOutputLabelsBug1Test {
    private val k340 = File("../../../tests/fixtures/preset/k340-bug1.dspipreset").readText()

    private fun demo() = FfiDspiCore().apply { openDemo() }

    @Test
    fun bug1K340LabelsAndCards() {
        demo().use { core ->
            val reply = core.configImport(k340, DspiConstants.IMPORT_HARDWARE.toLong())
            assertTrue(reply, reply.contains("\"ok\":true"))
            val labels = core.outputLabels()
            assertEquals(
                listOf("Out 1 · R · Low", "Out 2 · R · High", "Out 3 · L · Low", "Out 4 · L · High"),
                labels.take(4).map { it.label }
            )
            assertEquals(listOf("I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"), labels.take(4).map { it.slotName })
            // Review c86 F3: a phone's row beside the OUTn button
            assertEquals(listOf("R · Low", "R · High", "L · Low", "L · High"), labels.take(4).map { it.shortLabel })
            assertTrue(labels.none { it.userNamed })
            assertEquals(
                listOf(
                    OutputCard(2, 0, "Woofers", "Woofers L", "Woofers R", "Woofers L", "R"),
                    OutputCard(3, 1, "Tweeters", "Tweeters L", "Tweeters R", "Tweeters L", "R")
                ),
                core.outputCards()
            )

            // A user's name wins, as it is
            core.setChannelName(core.getState().numInputChannels + 0, "Right woofer")
            val named = core.outputLabels()[0]
            assertEquals("Right woofer", named.label)
            assertTrue(named.userNamed)
        }
    }

    @Test
    fun bug1DefaultDemo() {
        demo().use { core ->
            val s = core.getState()
            val labels = core.outputLabels()
            assertEquals(s.numOutputChannels, labels.size)
            labels.forEachIndexed { o, l -> assertTrue(l.label, l.userNamed || l.label.startsWith("Out ${o + 1} · ")) }
            // No pair map on the demo: the cards pair by slot
            core.outputCards().forEach { c -> c.right?.let { assertEquals(c.left + 1, it) } }
            assertFalse(s.crossfeedPairMap.any { it != 0 })
        }
    }
}
