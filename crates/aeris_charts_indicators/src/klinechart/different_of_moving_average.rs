//! `DMA` (平行线差). Ported from KLineChart
//! `src/extension/indicator/differentOfMovingAverage.ts`.

use super::Column;
use super::stepper::{Out, Window, fold, rolling_mean_step};

/// DMA outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Dma {
    /// `DIF = MA(CLOSE, N1) - MA(CLOSE, N2)`.
    pub dma: Column,
    /// `AMA = MA(DIF, M)`.
    pub ama: Column,
}

/// The two running sums of the rolling means of close: everything that produces `DIF`.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    short_sum: f64,
    long_sum: f64,
}

/// `lead` produces the DIF of the current row. The average of DIF drops the DIF `signal - 1` rows
/// back, so `shadow` is a second copy of the same stage run that far behind: it consumes the same
/// rows with the same arithmetic and hands back that DIF bit for bit, whatever the period. `sum` is
/// the running sum of the DIF values inside the average's window.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    lead: Stage,
    shadow: Stage,
    sum: f64,
}

/// The shadow reads the window of the longest mean `signal - 1` rows back, and that mean reads
/// `max(short, long) - 1` rows before its own row.
pub(super) fn lookback(short: usize, long: usize, signal: usize) -> usize {
    (signal + short.max(long)).saturating_sub(2)
}

/// Advances `stage` by the valid row `row` and returns that row's DIF, unset until both means exist.
fn dif_stage(
    stage: &mut Stage,
    w: &Window<'_>,
    row: usize,
    short: usize,
    long: usize,
) -> Option<f64> {
    let ma_short = rolling_mean_step(&mut stage.short_sum, row, short, w.c(row), || {
        w.c(row + 1 - short)
    });
    let ma_long = rolling_mean_step(&mut stage.long_sum, row, long, w.c(row), || {
        w.c(row + 1 - long)
    });
    ma_short.zip(ma_long).map(|(fast, slow)| fast - slow)
}

/// Advances the formula by the valid row `i`. A zero period leaves both outputs unset.
pub(super) fn step(
    short: usize,
    long: usize,
    signal: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if short == 0 || long == 0 || signal == 0 {
        return;
    }
    let dif = dif_stage(&mut st.lead, w, i, short, long);
    // The shadow consumes one row every tick, read or not, so it never misses a row.
    let leaving = if i + 1 >= signal {
        dif_stage(&mut st.shadow, w, i + 1 - signal, short, long)
    } else {
        None
    };
    let Some(dif) = dif else {
        return;
    };
    out[0] = Some(dif);
    // The average counts rows from the first DIF row, `max(short, long) - 1`.
    out[1] = rolling_mean_step(&mut st.sum, i + 1 - short.max(long), signal, dif, || {
        leaving.unwrap_or(0.0)
    });
}

/// KLineChart default: `short = 10`, `long = 50`, `signal = 10`.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn dma(close: &[f64], short: usize, long: usize, signal: usize) -> Dma {
    let window = Window {
        close,
        ..Window::EMPTY
    };
    let [dma, ama]: [Column; 2] = fold::<State>(close.len(), 2, |st, i, out| {
        step(short, long, signal, st, &window, i, out);
    })
    .try_into()
    .expect("a fold of two outputs has two columns");
    Dma { dma, ama }
}
