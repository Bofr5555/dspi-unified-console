package se.bofr.dspi.feature.home

import android.app.Application
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.material3.Surface
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.home.components.CrossoverRows
import se.bofr.dspi.feature.home.components.XO_SAME_TAG
import se.bofr.dspi.feature.home.components.XO_TYPE_TAG
import se.bofr.dspi.feature.home.components.XoWay
import se.bofr.dspi.feature.home.components.xoBandFor
import se.bofr.dspi.feature.home.components.xoChoices
import se.bofr.dspi.feature.home.components.xoShortName
import se.bofr.dspi.feature.home.components.xoWay
import se.bofr.dspi.ui.BandFields
import se.bofr.dspi.ui.CrossoverFamily
import se.bofr.dspi.ui.DspiTheme
import se.bofr.dspi.ui.FilterType

/**
 * WP UAN (UXP B10): an output's crossover in two rows: a way's band is the first of that way,
 * else a free one; LR24 at 2500 Hz from the row's menu is one write; "Same for the right side"
 * gives the partner the same crossover in one undo step.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class, qualifiers = "w400dp-h800dp-port-mdpi")
class CrossoverRowsWpUanTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val bridge = DspiBridgeImpl(FakeDspiCore.seeded(), scope, ffi = Dispatchers.Main, main = Dispatchers.Main)

    @After fun stop() = scope.cancel()

    private fun settle(until: () -> Boolean = { true }) {
        val end = System.currentTimeMillis() + 10_000
        var rounds = 0
        while (rounds < 10 || !until()) {
            check(System.currentTimeMillis() < end) { "timed out" }
            shadowOf(Looper.getMainLooper()).idle()
            rule.runOnUiThread { Snapshot.sendApplyNotifications() }
            rule.mainClock.advanceTimeBy(50)
            rounds++
        }
    }

    private val lr24Hp = CrossoverFamily.LINKWITZ_RILEY.filterType(4, lowPass = false)!!
    private val lr24Lp = CrossoverFamily.LINKWITZ_RILEY.filterType(4, lowPass = true)!!
    private fun off() = BandFields(FilterType.FLAT, 1000f, 0.707f, 0f, 0.707f)

    @Test
    fun uanTheWaysAndTheirBands() {
        assertEquals("LR24", xoShortName(lr24Hp))
        assertEquals("Off", xoShortName(FilterType.FLAT))
        assertEquals("BW6", xoShortName(CrossoverFamily.BUTTERWORTH.filterType(1, lowPass = true)!!))
        assertEquals(FilterType.FLAT, xoChoices(lowPass = false).first())
        assertTrue(xoChoices(lowPass = false).drop(1).all { it.crossoverIsLowPass == false })
        // Nothing set: both ways Off, the high-pass takes band 1, the low-pass then the next free one
        val none = List(4) { off() }
        val hp = xoWay(none, lowPass = false)
        assertEquals(XoWay(null, FilterType.FLAT, 2500f), hp)
        assertEquals(0, xoBandFor(none, hp, xoWay(none, lowPass = true)))
        val withHp = listOf(BandFields(lr24Hp, 2500f, 0.707f, 0f, 0.707f)) + List(3) { off() }
        val hp2 = xoWay(withHp, lowPass = false)
        val lp2 = xoWay(withHp, lowPass = true)
        assertEquals(0, hp2.band)
        assertNull(lp2.band)
        assertEquals("its own band", 0, xoBandFor(withHp, hp2, lp2))
        assertEquals("a free one", 1, xoBandFor(withHp, lp2, hp2))
        assertNull("none free", xoBandFor(List(4) { BandFields(lr24Hp, 2500f, 0.707f, 0f, 0.707f) }, lp2, hp2))
    }

    @Test
    fun uanTheRowsMenuSetsLr24AtTheCorner() {
        val xo = mutableStateOf(List(4) { off() })
        val writes = mutableListOf<Triple<Int, FilterType, Float>>()
        var same = 0
        rule.setContent {
            DspiTheme(darkTheme = true) {
                Surface {
                    CrossoverRows(
                        xo.value,
                        connected = true,
                        onSet = { b, t, f -> writes += Triple(b, t, f) },
                        partnerName = sameForWords(true, "Out 2 · R"),
                        onSameForPartner = { same++ }
                    )
                }
            }
        }
        rule.onNodeWithTag(XO_TYPE_TAG + "hp").performClick()
        rule.onNode(hasText("LR24")).performClick()
        assertEquals(listOf(Triple(0, lr24Hp, 2500f)), writes)
        rule.onNodeWithTag(XO_SAME_TAG).assert(hasText("Same for the right side")).performClick()
        assertEquals(1, same)
        assertEquals("Same for Out 3 · None", sameForWords(false, "Out 3 · None"))
    }

    @Test
    fun uanSameForTheRightSideIsOneUndoStep() {
        rule.mainClock.autoAdvance = false
        bridge.scanDevices()
        settle { bridge.status.value.connected }
        // Out 1 (app id 2): LR24 high-pass at 2500 Hz and LR24 low-pass at 18 kHz
        bridge.setCrossover(2, 0, lr24Hp.raw, 2500f)
        bridge.setCrossover(2, 1, lr24Lp.raw, 18000f)
        settle { bridge.channelBands(2)?.xover?.get(1)?.filterType == lr24Lp.raw }
        assertFalse(bridge.channelBands(3)!!.xover.any { it.filterType == lr24Hp.raw })
        bridge.copyCrossover(2, 3)
        settle { bridge.channelBands(3)?.xover?.get(1)?.filterType == lr24Lp.raw }
        val right = bridge.channelBands(3)!!.xover
        assertEquals(lr24Hp.raw, right[0].filterType)
        assertEquals(2500f, right[0].freq, 0.5f)
        assertEquals(18000f, right[1].freq, 1f)
        // One undo puts both bands back
        scope.launch { bridge.undo() }
        settle { bridge.channelBands(3)?.xover?.none { it.filterType == lr24Hp.raw || it.filterType == lr24Lp.raw } == true }
        assertEquals("the left side kept", lr24Hp.raw, bridge.channelBands(2)!!.xover[0].filterType)
    }
}
