//! WP GAP1: an output's polarity, as the matrix makes it: every route that
//! feeds the output inverted, none, some, or no route at all. Qt's Delay &
//! Gain row, routing inspector and channel strip show it
//! (`ChannelController::outputPolarity`); the web console reads it from its
//! snapshot (`outputPolarity`). Changing it is a write of each feeding
//! route's invert flag, as Qt's `setOutputPolarity` and the agents'
//! `dspi_set_polarity` do.

use serde::Serialize;

use crate::records::DspState;

/// An output's polarity from the routes that feed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputPolarity {
    /// No input feeds it.
    None,
    /// Every feeding route plays as it comes.
    Normal,
    /// Every feeding route is inverted.
    Inverted,
    /// Some are inverted, some are not.
    Mixed,
}

/// The polarity of output `output` (0-based) in `s`: over the live inputs'
/// routes to it (`matrix_routing`, `matrix_invert`).
pub fn output_polarity(s: &DspState, output: usize) -> OutputPolarity {
    let (mut fed, mut inverted) = (0usize, 0usize);
    for i in 0..usize::from(s.num_input_channels) {
        let routed = s
            .matrix_routing
            .get(i)
            .and_then(|r| r.get(output))
            .copied()
            .unwrap_or(false);
        if !routed {
            continue;
        }
        fed = fed.saturating_add(1);
        if s.matrix_invert
            .get(i)
            .and_then(|r| r.get(output))
            .copied()
            .unwrap_or(false)
        {
            inverted = inverted.saturating_add(1);
        }
    }
    match (fed, inverted) {
        (0, _) => OutputPolarity::None,
        (_, 0) => OutputPolarity::Normal,
        (f, n) if f == n => OutputPolarity::Inverted,
        _ => OutputPolarity::Mixed,
    }
}

/// Every live output's polarity, in output order.
pub fn output_polarities(s: &DspState) -> Vec<OutputPolarity> {
    (0..usize::from(s.num_output_channels))
        .map(|o| output_polarity(s, o))
        .collect()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests_wp_gap1 {
    use super::*;
    use crate::state::default_state;

    #[test]
    fn gap1_an_outputs_polarity_is_its_feeding_routes() {
        let mut s = default_state();
        s.num_input_channels = 3;
        s.num_output_channels = 4;
        for r in &mut s.matrix_routing {
            r.iter_mut().for_each(|x| *x = false);
        }
        // Out 1: fed by in 1, as it comes; Out 2: by in 1 and 2, both
        // inverted; Out 3: by in 1 (inverted) and in 3; Out 4: by nothing
        s.matrix_routing[0][0] = true;
        s.matrix_routing[0][1] = true;
        s.matrix_routing[1][1] = true;
        s.matrix_invert[0][1] = true;
        s.matrix_invert[1][1] = true;
        s.matrix_routing[0][2] = true;
        s.matrix_invert[0][2] = true;
        s.matrix_routing[2][2] = true;
        // An invert flag on a route that is off does not count
        s.matrix_invert[2][0] = true;
        s.matrix_invert[1][3] = true;
        assert_eq!(
            output_polarities(&s),
            [
                OutputPolarity::Normal,
                OutputPolarity::Inverted,
                OutputPolarity::Mixed,
                OutputPolarity::None
            ]
        );
        // Only the live inputs count: in 3 gone, Out 3 is inverted
        s.num_input_channels = 2;
        assert_eq!(output_polarity(&s, 2), OutputPolarity::Inverted);
        // Past the matrix: nothing feeds it
        assert_eq!(output_polarity(&s, 99), OutputPolarity::None);
        assert_eq!(
            serde_json::to_value(output_polarities(&s)).unwrap(),
            serde_json::json!(["normal", "inverted", "inverted", "none"])
        );
    }
}
