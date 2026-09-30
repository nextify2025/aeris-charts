//! Redundant-setter elimination for the browser Canvas2D target.
//!
//! A lowered dash run is one stroke per dash, and every setter the executor issues for a stroke
//! is a wasm-bindgen crossing (the color also allocates and parses a CSS string, the dash builds a
//! JS array), so repeating the previous stroke's state dominates a dashed line's cost. Every
//! setter is idempotent, so skipping one whose value is already in place cannot change a pixel.
//!
//! Every field starts unknown and [`StrokeState::invalidate`] forgets them again: the target calls
//! it on `restore`, which pops state the target did not set. Nothing else touches the context
//! while a target is alive (the chart builds one per frame and drives every draw through it), so
//! no other invalidation is needed. Pure and host-testable; the target does the JS calls.

use aeris_charts_render::color::Color;

/// The stroke state last applied to the context; each method returns whether the context still
/// needs the value set.
#[derive(Default)]
pub(crate) struct StrokeState {
    color: Option<Color>,
    width: Option<f32>,
    dash: Option<Vec<f32>>,
    round_join_butt_cap: bool,
}

impl StrokeState {
    pub(crate) fn needs_color(&mut self, color: Color) -> bool {
        self.color.replace(color) != Some(color)
    }

    pub(crate) fn needs_width(&mut self, width: f32) -> bool {
        self.width.replace(width) != Some(width)
    }

    pub(crate) fn needs_dash(&mut self, pattern: &[f32]) -> bool {
        if self.dash.as_deref() == Some(pattern) {
            return false;
        }
        let known = self.dash.get_or_insert_with(Vec::new);
        known.clear();
        known.extend_from_slice(pattern);
        true
    }

    /// The round-join, butt-cap pair every stroke uses. Host plugins share the context and may
    /// leave other join/cap state, so the first stroke sets it.
    pub(crate) fn needs_join_and_cap(&mut self) -> bool {
        !std::mem::replace(&mut self.round_join_butt_cap, true)
    }

    pub(crate) fn invalidate(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::rgb(0xff, 0x00, 0x00);
    const BLUE: Color = Color::rgb(0x00, 0x00, 0xff);

    #[test]
    fn a_repeated_stroke_state_sets_nothing_after_the_first() {
        let mut state = StrokeState::default();
        // Ten lowered dashes of one dotted line: same color, width, empty dash pattern.
        let mut sets = 0;
        for _ in 0..10 {
            sets += usize::from(state.needs_color(RED));
            sets += usize::from(state.needs_width(2.0));
            sets += usize::from(state.needs_dash(&[]));
            sets += usize::from(state.needs_join_and_cap());
        }
        assert_eq!(sets, 4, "each setter runs once for the whole line");
    }

    #[test]
    fn a_changed_value_is_set_again_and_only_that_one() {
        let mut state = StrokeState::default();
        assert!(state.needs_color(RED) && state.needs_width(2.0) && state.needs_dash(&[]));
        assert!(state.needs_color(BLUE));
        assert!(!state.needs_width(2.0));
        assert!(state.needs_width(3.0));
        assert!(!state.needs_color(BLUE));
        // Switching to a dashed pattern and back is two real changes, and a repeat is not.
        assert!(state.needs_dash(&[6.0, 24.0]));
        assert!(!state.needs_dash(&[6.0, 24.0]));
        assert!(state.needs_dash(&[6.0, 12.0]));
        assert!(state.needs_dash(&[]));
        assert!(!state.needs_dash(&[]));
    }

    #[test]
    fn restore_forgets_everything_the_target_applied() {
        let mut state = StrokeState::default();
        assert!(state.needs_color(RED));
        assert!(state.needs_width(2.0));
        assert!(state.needs_dash(&[1.0, 4.0]));
        assert!(state.needs_join_and_cap());
        state.invalidate();
        assert!(state.needs_color(RED));
        assert!(state.needs_width(2.0));
        assert!(state.needs_dash(&[1.0, 4.0]));
        assert!(state.needs_join_and_cap());
    }

    #[test]
    fn an_unrepresentable_width_is_always_applied() {
        // NaN never compares equal, so the context sees every request as it did before caching.
        let mut state = StrokeState::default();
        assert!(state.needs_width(f32::NAN));
        assert!(state.needs_width(f32::NAN));
    }
}
