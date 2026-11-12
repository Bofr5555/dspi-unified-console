pragma Singleton
import QtQuick

// -- WP QP9 -- The generator tiles' small waveforms (the Mac's glyphs): for a
// type id, a list of polylines, each a list of [x, v] with x in 0..1 and v in
// -1..1 (up is positive).
QtObject {
    function jitter(t) { return Math.sin(91.7 * t + 1.3) * 0.5 + Math.sin(173.3 * t) * 0.35 + Math.sin(47.9 * t + 4.1) * 0.15 }

    function sampled(f, n) {
        const pts = []
        for (let i = 0; i <= n; ++i) {
            const t = i / n
            pts.push([t, f(t)])
        }
        return pts
    }
    // Flat at 0 with lobes: each [from, to, peak] a triangle-free spike
    function spikes(list) {
        const pts = [[0, 0]]
        for (const s of list) {
            pts.push([s[0], 0])
            pts.push([(s[0] + s[1]) / 2, s[2]])
            pts.push([s[1], 0])
        }
        pts.push([1, 0])
        return pts
    }

    function paths(type) {
        const tau = 2 * Math.PI
        switch (type) {
        case 0: return [sampled(t => Math.sin(tau * 2 * t), 64)]
        case 1: {
            const pts = []
            for (let i = 0; i < 4; ++i) {
                const v = i % 2 === 0 ? 0.8 : -0.8
                pts.push([i / 4, v], [(i + 1) / 4, v])
            }
            return [pts]
        }
        case 2: return [sampled(t => jitter(t) * 1.5, 40)]
        case 3: return [sampled(t => Math.sin(tau * 1.3 * t) * 0.7 + jitter(t) * 0.5, 40)]
        case 4: return [sampled(t => Math.sin(tau * 1.2 * (Math.pow(6, t) - 1)), 64)]
        case 5: return [sampled(t => Math.sin(tau * (1 + 4 * t) * t), 64)]
        case 6: {
            const out = []
            for (let i = 0; i < 4; ++i) {
                const v = i / 3 * 1.6 - 0.8
                out.push([[i / 4, v], [(i + 1) / 4 - 0.04, v]])
            }
            return out
        }
        case 7: return [spikes([[0.45, 0.51, 0.96]])]
        case 8: return [spikes([[0.28, 0.34, 0.96], [0.64, 0.70, -0.96]])]
        case 9: return [[[0, 0], [0.3, 0]].concat(sampled(t => Math.sin(Math.PI * t), 16).map(p => [0.3 + p[0] * 0.4, p[1] * 0.9])).concat([[1, 0]])]
        case 10: return [sampled(t => (t >= 0.15 && t <= 0.55 ? Math.sin(Math.PI * (t - 0.15) / 0.4) : 0) * Math.sin(tau * 6 * t), 64)]
        case 11: return [sampled(t => 0.72 * Math.sin(tau * 1.5 * t) + 0.28 * Math.sin(tau * 11 * t), 64)]
        case 12: return [sampled(t => (Math.sin(tau * 1.5 * t) + Math.sin(tau * 3.7 * t + 1) + Math.sin(tau * 7.3 * t + 2)) / 2.6, 64)]
        case 13: {
            // Two flat runs and the over between them, a curve up past the top
            const over = sampled(t => 0.7 + 4 * t * (1 - t) * 0.45, 16).map(p => [0.05 + p[0] * 0.4, p[1]])
            return [over, [[0.05, 0.7], [0.45, 0.7]], [[0.55, -0.7], [0.95, -0.7]]]
        }
        case 14: {
            const out = []
            const centres = [0.18, 0.5, 0.82]
            const amps = [0.45, 0.72, 0.99]
            for (let b = 0; b < 3; ++b) {
                const pts = []
                for (let i = 0; i <= 12; ++i) {
                    const ph = i / 12
                    pts.push([centres[b] - 0.11 + ph * 0.22, Math.sin(Math.PI * ph) * amps[b] * Math.sin(6 * Math.PI * ph)])
                }
                out.push(pts)
            }
            return out
        }
        }
        return []
    }
}
