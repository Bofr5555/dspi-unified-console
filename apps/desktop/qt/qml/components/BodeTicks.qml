pragma Singleton
import QtQuick

// -- INTC (DL8 fold) -- The ticks of a Bode graph's axes as AxisLabels takes
// them ({at: 0..1 along the axis, text}), one set for every such graph in
// Modern (GraphPlot, Equaliser › Crossover; Measure and AutoEQ when they use
// a GraphFrame), so they read the same:
//   frequency(min, max, shown)     1-2-5 per decade, 20 Hz to 20 kHz, "1k"
//   db(top, bottom, span, shown)   a whole step for the span, "+6", "-12"
//   phase(extent, shown)           ±extent, ±extent/2 and 0, in degrees
//
//     GraphFrame {
//         xTicks: BodeTicks.frequency(GraphController.minFreq, GraphController.maxFreq, true)
//     }
QtObject {
    function frequency(min, max, shown) {
        const out = []
        if (!shown || !(min > 0) || !(max > min)) return out
        for (const decade of [10, 100, 1000, 10000])
            for (const k of [1, 2, 5]) {
                const hz = k * decade
                if (hz < 20 || hz > 20000 || hz < min || hz > max) continue
                out.push({ at: Math.log10(hz / min) / Math.log10(max / min), text: hz < 1000 ? String(hz) : (hz / 1000) + "k" })
            }
        return out
    }
    function db(top, bottom, span, shown) {
        const out = []
        if (!shown || !(top > bottom)) return out
        const step = span <= 12 ? 1 : span <= 30 ? 3 : span <= 60 ? 5 : 10
        for (let k = Math.ceil(bottom / step); k * step <= top; ++k) {
            const v = k * step
            out.push({ at: (top - v) / (top - bottom), text: (k >= 0 ? "+" : "") + v })
        }
        return out
    }
    function phase(extent, shown) {
        if (!shown) return []
        return [1, 0.5, 0, -0.5, -1].map(share => {
            const deg = Math.round(share * extent)
            return { at: (1 - share) / 2, text: deg === 0 ? "0°" : (deg > 0 ? "+" : "") + deg + "°" }
        })
    }
}
