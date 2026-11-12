//! WP CS1: `REQ_GET_SUBHARM_HEADROOM`, as `subharm_headroom_db` in the
//! firmware's `subharm.c` (bofr-main de9614c) computes it from the live
//! config: the worst-case gain of a steady tone over the bass band, scanned
//! on an eighth-octave grid with analog-prototype magnitudes, and bisected
//! against the ceiling when one is set. 0 dB when the effect is off.

use dspi_proto::generated::ranges as sh;

/// Fourier amplitudes of the divided waveform at odd multiples of the sub
/// frequency (`sh_div_harm`).
const DIV_HARM: [f32; 4] = [0.8488, 0.5093, 0.1213, 0.0566];
/// The scan: 16 Hz up in 2^(1/8) steps, 37 points (to 362 Hz).
const SCAN_F0: f32 = 16.0;
const SCAN_STEP: f32 = 1.090_507_7;
const SCAN_POINTS: usize = 37;
/// The sub lowpass of each band (`sh_sub_lp_hz`).
const SUB_LP_HZ: [f32; 3] = [
    sh::SUBHARM_SUB_LP0_HZ,
    sh::SUBHARM_SUB_LP1_HZ,
    sh::SUBHARM_SUB_LP2_HZ,
];

/// The config fields the bound reads.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Config {
    pub enabled: bool,
    pub low_db: f32,
    pub high_db: f32,
    pub top_db: f32,
    pub boost_db: f32,
    pub ceiling_db: f32,
}

fn lp2(w: f32) -> f32 {
    let w2 = w * w;
    1.0 / (1.0 + w2 * w2).sqrt()
}

fn hp2(w: f32) -> f32 {
    let w2 = w * w;
    w2 / (1.0 + w2 * w2).sqrt()
}

fn bell(w: f32, amp: f32) -> f32 {
    if amp <= 1.0 {
        return 1.0;
    }
    let w2 = w * w;
    let d = (1.0 - w2) * (1.0 - w2);
    let num = w * amp / sh::SUBHARM_BOOST_Q;
    let den = w / (amp * sh::SUBHARM_BOOST_Q);
    ((d + num * num) / (d + den * den)).sqrt()
}

fn level_gain(db: f32) -> f32 {
    let db = db.clamp(sh::SUBHARM_LEVEL_MIN, sh::SUBHARM_LEVEL_MAX);
    if db <= sh::SUBHARM_LEVEL_MIN {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

fn boost_amp(db: f32) -> f32 {
    let db = db.clamp(sh::SUBHARM_BOOST_MIN, sh::SUBHARM_BOOST_MAX);
    if db <= 0.0 {
        1.0
    } else {
        10f32.powf(db / 40.0)
    }
}

/// The headroom in dB the firmware would report for `c`.
pub(super) fn headroom_db(c: Config) -> f32 {
    if !c.enabled {
        return 0.0;
    }
    let gain = [
        level_gain(c.low_db),
        level_gain(c.high_db),
        level_gain(c.top_db),
    ];
    let amp = boost_amp(c.boost_db);
    let ceil_db = c
        .ceiling_db
        .clamp(sh::SUBHARM_CEILING_MIN, sh::SUBHARM_CEILING_MAX);
    let ceil_lin = if ceil_db < 0.0 {
        10f32.powf(ceil_db / 20.0)
    } else {
        0.0
    };

    // Per scan point: the direct tone through the bell, and the sub sum with
    // and without the bell.
    let mut points = Vec::with_capacity(SCAN_POINTS);
    let mut f = SCAN_F0;
    for _ in 0..SCAN_POINTS {
        let pre = hp2(f / sh::SUBHARM_BAND_LO_HZ) * lp2(f / sh::SUBHARM_BAND_HI_HZ);
        let lo2 = pre * lp2(f / sh::SUBHARM_BAND_MID2_HZ);
        let band_mag = [
            lo2 * lp2(f / sh::SUBHARM_BAND_MID1_HZ),
            lo2 * hp2(f / sh::SUBHARM_BAND_MID1_HZ),
            pre * hp2(f / sh::SUBHARM_BAND_MID2_HZ),
        ];
        let (mut flat_sum, mut bell_sum) = (0.0f32, 0.0f32);
        for ((g, mag), lp_hz) in gain.iter().zip(band_mag).zip(SUB_LP_HZ) {
            if *g <= 0.0 {
                continue;
            }
            let (mut flat, mut belled) = (0.0f32, 0.0f32);
            // The odd multiples 1, 3, 5, 7 of the sub frequency.
            for (n, harm) in [1.0f32, 3.0, 5.0, 7.0].into_iter().zip(DIV_HARM) {
                let fn_ = n * 0.5 * f;
                let m = harm * lp2(fn_ / lp_hz);
                flat += m;
                belled += m * bell(fn_ / sh::SUBHARM_BOOST_HZ, amp);
            }
            flat_sum += g * mag * flat;
            bell_sum += g * mag * belled;
        }
        points.push((bell(f / sh::SUBHARM_BOOST_HZ, amp), flat_sum, bell_sum));
        f *= SCAN_STEP;
    }

    let peak_max = points
        .iter()
        .map(|(direct, _, sub_bell)| direct + sub_bell)
        .fold(1.0f32, f32::max);
    if ceil_lin <= 0.0 {
        return 20.0 * peak_max.log10();
    }
    // With a ceiling: the smallest headroom at which a tone cannot pass full
    // scale, by bisection on the input level.
    let (mut g_lo, mut g_hi) = (1.0 / peak_max, 1.0f32);
    for _ in 0..24 {
        let g = 0.5 * (g_lo + g_hi);
        let worst = points
            .iter()
            .map(|&(direct, sub_flat, sub_bell)| {
                let sub = if g * sub_flat > ceil_lin {
                    ceil_lin * sub_bell / sub_flat
                } else {
                    g * sub_bell
                };
                g * direct + sub
            })
            .fold(0.0f32, f32::max);
        if worst > 1.0 {
            g_hi = g;
        } else {
            g_lo = g;
        }
    }
    -20.0 * g_lo.log10()
}

#[cfg(test)]
mod tests_wp_cs1 {
    use super::*;

    fn on() -> Config {
        Config {
            enabled: true,
            low_db: 0.0,
            high_db: 0.0,
            top_db: sh::SUBHARM_LEVEL_MIN,
            boost_db: 0.0,
            ceiling_db: 0.0,
        }
    }

    #[test]
    fn wp_cs1_off_is_zero_and_louder_settings_need_more_headroom() {
        assert_eq!(headroom_db(Config::default()), 0.0);
        let base = headroom_db(on());
        assert!(base > 0.0 && base < 20.0, "{base}");
        let louder = headroom_db(Config {
            low_db: 6.0,
            ..on()
        });
        assert!(louder > base, "{louder} vs {base}");
        let boosted = headroom_db(Config {
            boost_db: 6.0,
            ..on()
        });
        assert!(boosted > base, "{boosted} vs {base}");
        // Every band at the floor: only the direct tone, 0 dB.
        let silent = headroom_db(Config {
            low_db: sh::SUBHARM_LEVEL_MIN,
            high_db: sh::SUBHARM_LEVEL_MIN,
            ..on()
        });
        assert_eq!(silent, 0.0);
    }

    #[test]
    fn wp_cs1_a_ceiling_never_needs_more_headroom() {
        let loud = Config {
            low_db: 12.0,
            high_db: 12.0,
            ..on()
        };
        let free = headroom_db(loud);
        let capped = headroom_db(Config {
            ceiling_db: -20.0,
            ..loud
        });
        assert!(capped <= free + 1e-3, "{capped} vs {free}");
        assert!(capped >= 0.0);
    }

    #[test]
    fn wp_cs1_the_demo_answers_the_headroom_from_its_config() {
        use dspi_proto::generated::opcodes as op;

        use crate::{FakeDevice, Transport};

        let mut d = FakeDevice::demo();
        let read = |d: &mut FakeDevice| {
            let b = d.control_in(op::REQ_GET_SUBHARM_HEADROOM, 0, 4).unwrap();
            f32::from_le_bytes(b.try_into().unwrap())
        };
        d.control_out(op::REQ_SET_SUBHARM, 0, &[0]).unwrap();
        assert_eq!(read(&mut d), 0.0, "off");
        d.control_out(op::REQ_SET_SUBHARM, 0, &[1]).unwrap();
        let on = read(&mut d);
        d.control_out(op::REQ_SET_SUBHARM_LOW, 0, &12f32.to_le_bytes())
            .unwrap();
        assert!(read(&mut d) > on, "{on}");
    }
}
