//! WP GAP1: the crossfeed's output pair map for the web (Qt's
//! `ExtController` and `CrossfeedPairMap.qml`): which two outputs each
//! crossfeed pair processes, on firmware with the extension's pair map. The
//! rules are the core's (`dspi_proto::ext::PairMap`,
//! `dspi_core::crossfeed_pair_map_valid`); the write is
//! `DspiCore::set_crossfeed_pair_map` (checked, read back, one undo step).
//!
//! - `crossfeedPairMap`: `{supported, pairCount, pairs: [{left, right}],
//!   choice, k340Mask}`. `pairs` are 0-based outputs from the mirror,
//!   `choice` is `default` (1/2, 3/4, …), `k340` (one DAC per ear) or
//!   `custom`; `k340Mask` the pair mask that goes with the K340 map.
//! - `crossfeedPairsValid {pairs}`: whether the firmware would take these
//!   pairs (one per crossfeed pair, two different outputs each, no output
//!   in two pairs).
//! - `setCrossfeedPairMap {choice}` (`default` or `k340`) or `{pairs}`:
//!   write it; answers `crossfeedPairMap` after the read-back.

use dspi_core::{DspiCore, DspiError, crossfeed_pair_map_valid};
use dspi_proto::ext::PairMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// One crossfeed pair's outputs, 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Pair {
    pub left: u8,
    pub right: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    supported: bool,
    pair_count: u8,
    pairs: Vec<Pair>,
    choice: &'static str,
    k340_mask: u8,
}

#[derive(Deserialize)]
struct Pairs {
    pairs: Vec<Pair>,
}

#[derive(Deserialize)]
struct Set {
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    pairs: Option<Vec<Pair>>,
}

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "crossfeedPairMap" => Ok(tidy(&view(core))),
        "crossfeedPairsValid" => {
            params::<Pairs>(p).map(|q| Value::Bool(encode(&q.pairs, pair_count(core)).is_some()))
        }
        "setCrossfeedPairMap" => params::<Set>(p).and_then(|q| set(core, q)),
        _ => return None,
    })
}

/// The crossfeed's stereo pairs: two on RP2040, four on RP2350.
fn pair_count(core: &DspiCore) -> u8 {
    if core.get_state().platform_id == 0 {
        2
    } else {
        4
    }
}

fn map_of(core: &DspiCore) -> PairMap {
    let mut raw = [0u8; PairMap::SIZE];
    for (d, s) in raw.iter_mut().zip(core.get_state().crossfeed_pair_map) {
        *d = s;
    }
    PairMap(raw)
}

fn view(core: &DspiCore) -> View {
    let supported = core.ext_status().is_ok_and(|s| s.has_pair_map);
    let n = pair_count(core);
    let map = if supported {
        map_of(core)
    } else {
        PairMap([0; PairMap::SIZE])
    };
    let pairs = (0..usize::from(n))
        .map(|p| {
            let (left, right) = map.pair(p);
            Pair { left, right }
        })
        .collect();
    let mine = map.normalized();
    let choice = if mine == PairMap([0; PairMap::SIZE]) {
        "default"
    } else if mine == PairMap::K340.normalized() {
        "k340"
    } else {
        "custom"
    };
    View {
        supported,
        pair_count: n,
        pairs,
        choice,
        k340_mask: PairMap::K340_MASK,
    }
}

/// `pairs` as the 4-byte map (a default pair as 0), or `None` when the
/// firmware would refuse it on `n` pairs (Qt's `ExtController::encode`).
fn encode(pairs: &[Pair], n: u8) -> Option<Vec<u8>> {
    if pairs.len() != usize::from(n) {
        return None;
    }
    let mut map = vec![0u8; PairMap::SIZE];
    for (p, (pair, byte)) in pairs.iter().zip(map.iter_mut()).enumerate() {
        if pair.left == pair.right || pair.left > 15 || pair.right > 15 {
            return None;
        }
        let first = u8::try_from(p).ok()?.checked_mul(2)?;
        *byte = if pair.left == first && Some(pair.right) == first.checked_add(1) {
            0
        } else {
            pair.right.wrapping_shl(4) | pair.left
        };
    }
    crossfeed_pair_map_valid(map.clone(), n).then_some(map)
}

fn set(core: &DspiCore, q: Set) -> CallResult {
    let map = match (q.choice.as_deref(), q.pairs) {
        (Some("default"), None) => vec![0u8; PairMap::SIZE],
        (Some("k340"), None) => PairMap::K340.0.to_vec(),
        (None, Some(pairs)) => encode(&pairs, pair_count(core)).ok_or_else(|| {
            CallError::from(DspiError::InvalidArgument {
                msg: "each pair needs two different outputs, and no output can be in two pairs"
                    .to_owned(),
            })
        })?,
        _ => {
            return Err(CallError::new(
                "invalid",
                "give a choice (default or k340) or the pairs",
            ));
        }
    };
    core.set_crossfeed_pair_map(map)?;
    Ok(tidy(&view(core)))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
#[path = "pair_map/tests_wp_gap1.rs"]
mod tests_wp_gap1;
