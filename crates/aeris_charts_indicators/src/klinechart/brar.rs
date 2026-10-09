//! `BRAR` (情绪指标). Ported from KLineChart `src/extension/indicator/brar.ts`.

use super::Column;
use super::stepper::{Out, Window, fold};

/// BRAR outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Brar {
    /// `BR = SUM(HIGH - REF(CLOSE, 1), N) / SUM(REF(CLOSE, 1) - LOW, N) * 100`, 0 when the divisor
    /// is 0.
    pub br: Column,
    /// `AR = SUM(HIGH - OPEN, N) / SUM(OPEN - LOW, N) * 100`, 0 when the divisor is 0.
    pub ar: Column,
}

/// The four running sums of the window. A running add/subtract sum cannot be recomputed from its
/// window bit for bit, so they live in the checkpointed state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    hcy: f64,
    cyl: f64,
    ho: f64,
    ol: f64,
}

/// The leaving row's previous close is `period` rows back.
pub(super) fn lookback(period: usize) -> usize {
    period
}

/// Advances the formula by the valid row `i`. A zero period leaves both outputs unset. The first
/// row uses its own close as the previous close.
pub(super) fn step(period: usize, st: &mut State, w: &Window<'_>, i: usize, out: &mut Out) {
    if period == 0 {
        return;
    }
    let pc = w.c(i.saturating_sub(1));
    st.ho += w.h(i) - w.o(i);
    st.ol += w.o(i) - w.l(i);
    st.hcy += w.h(i) - pc;
    st.cyl += pc - w.l(i);
    if i + 1 >= period {
        out[1] = Some(if st.ol != 0.0 {
            (st.ho / st.ol) * 100.0
        } else {
            0.0
        });
        out[0] = Some(if st.cyl != 0.0 {
            (st.hcy / st.cyl) * 100.0
        } else {
            0.0
        });
        let ago = i + 1 - period;
        // KLineChart reads `dataList[i - N] ?? dataList[i - (N - 1)]`.
        let ago_prev_close = if i >= period {
            w.c(i - period)
        } else {
            w.c(ago)
        };
        st.hcy -= w.h(ago) - ago_prev_close;
        st.cyl -= ago_prev_close - w.l(ago);
        st.ho -= w.h(ago) - w.o(ago);
        st.ol -= w.o(ago) - w.l(ago);
    }
}

/// KLineChart default: `period = 26`. The first row uses its own close as the previous close.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn brar(open: &[f64], high: &[f64], low: &[f64], close: &[f64], period: usize) -> Brar {
    let window = Window {
        open,
        high,
        low,
        close,
        ..Window::EMPTY
    };
    let [br, ar]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(period, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of two outputs has two columns");
    Brar { br, ar }
}
