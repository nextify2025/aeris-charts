//! `SAR` (抛物线指标). Ported from KLineChart `src/extension/indicator/stopAndReverse.ts`.

use super::stepper::{fold, Out, Window};
use super::Column;

/// The parabolic state machine between rows: acceleration factor, extreme point, trend direction
/// and the previous SAR. The first row initialises it, so the default is never read.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    af: f64,
    ep: f64,
    is_increasing: bool,
    sar: f64,
}

/// A row reads the high and low one valid row before it.
pub(super) fn lookback(_start: f64, _step: f64, _max: f64) -> usize {
    1
}

/// Advances the state machine by the valid row `i`. The first row uses itself as the previous row.
pub(super) fn step(
    start: f64,
    step: f64,
    max: f64,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    const UNSET: f64 = -100.0;
    let start_af = start / 100.0;
    let step = step / 100.0;
    let max_af = max / 100.0;
    if i == 0 {
        *st = State {
            af: start_af,
            ep: UNSET,
            is_increasing: false,
            sar: 0.0,
        };
    }
    let prev_sar = st.sar;
    let prev = i.max(1) - 1;
    if st.is_increasing {
        if st.ep == UNSET || st.ep < w.h(i) {
            st.ep = w.h(i);
            st.af = (st.af + step).min(max_af);
        }
        st.sar = prev_sar + st.af * (st.ep - prev_sar);
        let low_min = w.l(prev).min(w.l(i));
        if st.sar > w.l(i) {
            st.sar = st.ep;
            st.af = start_af;
            st.ep = UNSET;
            st.is_increasing = !st.is_increasing;
        } else if st.sar > low_min {
            st.sar = low_min;
        }
    } else {
        if st.ep == UNSET || st.ep > w.l(i) {
            st.ep = w.l(i);
            st.af = (st.af + step).min(max_af);
        }
        st.sar = prev_sar + st.af * (st.ep - prev_sar);
        let high_max = w.h(prev).max(w.h(i));
        if st.sar < w.h(i) {
            st.sar = st.ep;
            st.af = start_af;
            st.ep = UNSET;
            st.is_increasing = !st.is_increasing;
        } else if st.sar < high_max {
            st.sar = high_max;
        }
    }
    out[0] = Some(st.sar);
}

/// Parabolic SAR with KLineChart's percent-unit parameters. KLineChart default: `start = 2`,
/// `step = 2`, `max = 20`, meaning an acceleration factor starting at 0.02, stepping by 0.02, and
/// capped at 0.20.
///
/// Every row has a value. KLineChart colors a dot with its up color when `SAR < (HIGH + LOW) / 2`,
/// otherwise with its down color; that is presentation and is left to the renderer.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn sar(high: &[f64], low: &[f64], start: f64, step: f64, max: f64) -> Column {
    let window = Window {
        high,
        low,
        ..Window::EMPTY
    };
    fold::<State>(high.len(), 1, |st, i, out| {
        self::step(start, step, max, st, &window, i, out);
    })
    .swap_remove(0)
}
