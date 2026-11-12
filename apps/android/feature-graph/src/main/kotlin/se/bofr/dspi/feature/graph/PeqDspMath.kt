// The graph editor's band response, in Kotlin after the macOS console's
// DSPMath.swift (DSPi-Console, GPL-3.0): RBJ biquads for the PEQ shapes, the
// tan-prewarped Linkwitz Transform, and the crossover cascades built from
// analog prototype poles through the bilinear transform, as the firmware
// designs them (the Bessel poles are the firmware's table). The editor needs
// the response synchronously, per frame and per touch, on the main thread,
// which a call into the core cannot give.
package se.bofr.dspi.feature.graph

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt
import kotlin.math.tan
import se.bofr.dspi.core.FilterParams

internal object PeqDspMath {
    /** The rate the graph is drawn at, as the macOS console. */
    const val SAMPLE_RATE: Double = 48000.0

    /** The firmware's lowest Q (FilterParams.qRange on macOS). */
    private const val Q_MIN = 0.1

    /** One section, normalised so a0 = 1. */
    private class Section(val b0: Double, val b1: Double, val b2: Double, val a1: Double, val a2: Double)

    private val FLAT = Section(1.0, 0.0, 0.0, 0.0, 0.0)

    /** A left-half-plane analog pole (-sigma ± j·omega); `single` is a real pole at -sigma. */
    private class Pole(val sigma: Double, val omega: Double, val single: Boolean = false)

    /**
     * The response in dB at [freq] of [filters] in cascade. Bypassed and
     * flat (type 0) bands add nothing.
     */
    fun responseAt(freq: Float, filters: List<FilterParams>): Float {
        val phi = sin(PI * freq / SAMPLE_RATE).let { it * it }
        var power = 1.0
        for (f in filters) {
            if (f.filterType == 0 || f.bypass) continue
            for (s in sections(f)) power *= powerGain(s, phi)
        }
        return (10.0 * log10(power)).toFloat()
    }

    private fun sections(f: FilterParams): List<Section> =
        if (f.filterType in 32..63) crossover(f.filterType, f.freq.toDouble()) else listOf(peq(f))

    /**
     * |H|² written in phi = sin²(w/2), which keeps narrow low sections exact
     * where 1 + a1·cos w + a2·cos 2w would cancel away.
     */
    private fun powerGain(s: Section, phi: Double): Double {
        val sumB = s.b0 + s.b1 + s.b2
        val sumA = 1.0 + s.a1 + s.a2
        val num = sumB * sumB - 4.0 * (s.b0 * s.b1 + s.b1 * s.b2 + 4.0 * s.b0 * s.b2) * phi +
            16.0 * s.b0 * s.b2 * phi * phi
        val den = sumA * sumA - 4.0 * (s.a1 + s.a1 * s.a2 + 4.0 * s.a2) * phi + 16.0 * s.a2 * phi * phi
        return if (den > 0.0) max(num, 0.0) / den else 1.0
    }

    // ── PEQ shapes (filter types 1..13) ─────────────────────────────────

    private fun peq(f: FilterParams): Section {
        if (f.filterType == 11) return linkwitzTransform(f)
        val w = 2.0 * PI * f.freq / SAMPLE_RATE
        val sn = sin(w)
        val cs = cos(w)
        val alpha = sn / (2.0 * max(f.q.toDouble(), Q_MIN))
        val a = 10.0.pow(f.gain / 40.0)
        val rootA = sqrt(a)
        // (b0, b1, b2, a0, a1, a2) before normalising.
        val c: DoubleArray = when (f.filterType) {
            1 -> doubleArrayOf(1 + alpha * a, -2 * cs, 1 - alpha * a, 1 + alpha / a, -2 * cs, 1 - alpha / a)
            2 -> doubleArrayOf(
                a * ((a + 1) - (a - 1) * cs + 2 * rootA * alpha),
                2 * a * ((a - 1) - (a + 1) * cs),
                a * ((a + 1) - (a - 1) * cs - 2 * rootA * alpha),
                (a + 1) + (a - 1) * cs + 2 * rootA * alpha,
                -2 * ((a - 1) + (a + 1) * cs),
                (a + 1) + (a - 1) * cs - 2 * rootA * alpha
            )
            3 -> doubleArrayOf(
                a * ((a + 1) + (a - 1) * cs + 2 * rootA * alpha),
                -2 * a * ((a - 1) + (a + 1) * cs),
                a * ((a + 1) + (a - 1) * cs - 2 * rootA * alpha),
                (a + 1) - (a - 1) * cs + 2 * rootA * alpha,
                2 * ((a - 1) - (a + 1) * cs),
                (a + 1) - (a - 1) * cs - 2 * rootA * alpha
            )
            4 -> doubleArrayOf((1 - cs) / 2, 1 - cs, (1 - cs) / 2, 1 + alpha, -2 * cs, 1 - alpha)
            5 -> doubleArrayOf((1 + cs) / 2, -(1 + cs), (1 + cs) / 2, 1 + alpha, -2 * cs, 1 - alpha)
            6 -> doubleArrayOf(1.0, -2 * cs, 1.0, 1 + alpha, -2 * cs, 1 - alpha)
            7 -> doubleArrayOf(1 - alpha, -2 * cs, 1 + alpha, 1 + alpha, -2 * cs, 1 - alpha)
            8 -> {
                // First-order all-pass: flat magnitude.
                val t = tan(w / 2)
                val k = (t - 1) / (t + 1)
                doubleArrayOf(k, 1.0, 0.0, 1.0, k, 0.0)
            }
            9 -> doubleArrayOf(a * sn + 1 + cs, a * sn - 1 - cs, 0.0, sn / a + 1 + cs, sn / a - 1 - cs, 0.0)
            10 -> doubleArrayOf(sn + a + a * cs, sn - a - a * cs, 0.0, sn + 1 / a + cs / a, sn - 1 / a - cs / a, 0.0)
            12 -> doubleArrayOf(sn, sn, 0.0, sn + 1 + cs, sn - 1 - cs, 0.0)
            13 -> doubleArrayOf(1 + cs, -1 - cs, 0.0, sn + 1 + cs, sn - 1 - cs, 0.0)
            else -> return FLAT
        }
        val a0 = c[3]
        return Section(c[0] / a0, c[1] / a0, c[2] / a0, c[4] / a0, c[5] / a0)
    }

    /**
     * Zeros at the driver's corner (f0 = freq, Q0 = q), poles at the target
     * (fp = gain in Hz, Qp = qp), both prewarped so the digital response is
     * exact at f0 and fp. A target at or below 0 Hz leaves the band flat.
     */
    private fun linkwitzTransform(f: FilterParams): Section {
        val f0 = f.freq.toDouble()
        val fp = f.gain.toDouble()
        if (f0 <= 0.0 || fp <= 0.0) return FLAT
        fun quad(fc: Double, q: Double): DoubleArray {
            val g = tan(PI * fc / SAMPLE_RATE)
            val qq = max(q, Q_MIN)
            return doubleArrayOf(1 + g / qq + g * g, 2 * (g * g - 1), 1 - g / qq + g * g)
        }
        val zeros = quad(f0, f.q.toDouble())
        val poles = quad(fp, f.qp.toDouble())
        val n = poles[0]
        return Section(zeros[0] / n, zeros[1] / n, zeros[2] / n, poles[1] / n, poles[2] / n)
    }

    // ── Crossovers (filter types 32..63) ────────────────────────────────

    /**
     * The cascade for crossover [type] at [fc]: Linkwitz-Riley 32..39 (orders
     * 2, 4, 6, 8), Butterworth 40..55 (1..8), Bessel 56..63 (2, 4, 6, 8); even
     * codes are low-pass.
     */
    private fun crossover(type: Int, fc: Double): List<Section> {
        if (fc <= 0.0) return emptyList()
        val lowPass = type % 2 == 0
        val poles = when (type) {
            // LR(2n) is Butterworth(n) twice.
            in 32..39 -> butterworth((type - 32) / 2 + 1).let { it + it }
            in 40..55 -> butterworth((type - 40) / 2 + 1)
            in 56..63 -> BESSEL[(type - 56) / 2]
            else -> return emptyList()
        }
        val wc = 2.0 * SAMPLE_RATE * tan(PI * fc / SAMPLE_RATE)
        return poles.map { bilinear(it, wc, lowPass) }
    }

    /** Butterworth poles of [order] on the unit circle: one real pole when odd, then the pairs. */
    private fun butterworth(order: Int): List<Pole> {
        val out = ArrayList<Pole>()
        if (order % 2 == 1) out += Pole(1.0, 0.0, single = true)
        for (k in 1..order / 2) {
            val angle = (order - 2 * k + 1) * PI / (2.0 * order)
            out += Pole(cos(angle), sin(angle))
        }
        return out
    }

    /** Bessel pole pairs, -3 dB at omega = 1, orders 2, 4, 6, 8 (the firmware's table). */
    private val BESSEL: List<List<Pole>> = listOf(
        listOf(Pole(1.10160, 0.63601)),
        listOf(Pole(1.37007, 0.41025), Pole(0.99521, 1.25711)),
        listOf(Pole(1.57149, 0.32090), Pole(1.38186, 0.97147), Pole(0.93066, 1.66186)),
        listOf(Pole(1.75741, 0.27287), Pole(1.63694, 0.82280), Pole(1.37384, 1.38836), Pole(0.89287, 1.99833))
    )

    /** One analog section scaled to [wc], through s = K(1 - z⁻¹)/(1 + z⁻¹). */
    private fun bilinear(pole: Pole, wc: Double, lowPass: Boolean): Section {
        val k = 2.0 * SAMPLE_RATE
        if (pole.single) {
            val p = pole.sigma * wc
            val a0 = k + p
            val a1 = (p - k) / a0
            return if (lowPass) Section(p / a0, p / a0, 0.0, a1, 0.0) else Section(k / a0, -k / a0, 0.0, a1, 0.0)
        }
        // A high-pass section is the low-pass one with s → wc²/s, which
        // moves the pole to (sigma, omega) / r².
        val r2 = pole.sigma * pole.sigma + pole.omega * pole.omega
        val sigma = if (lowPass) pole.sigma else pole.sigma / r2
        val omega = if (lowPass) pole.omega else pole.omega / r2
        val m = (sigma * sigma + omega * omega) * wc * wc
        val d = 2.0 * sigma * wc * k
        val kk = k * k
        val a0 = kk + d + m
        val a1 = (2.0 * m - 2.0 * kk) / a0
        val a2 = (kk - d + m) / a0
        // Unity pass-band gain: m(1 + z⁻¹)² for low-pass, K²(1 - z⁻¹)² for high-pass.
        val g = (if (lowPass) m else kk) / a0
        return Section(g, if (lowPass) 2.0 * g else -2.0 * g, g, a1, a2)
    }
}
