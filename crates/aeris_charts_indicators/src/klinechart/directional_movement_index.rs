//! `DMI` (趋向指标). Ported from KLineChart
//! `src/extension/indicator/directionalMovementIndex.ts`.

use super::stepper::{fold, Out, Window};
use super::Column;

/// DMI outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Dmi {
    /// `PDI = DMP * 100 / MTR`.
    pub pdi: Column,
    /// `MDI = DMM * 100 / MTR`.
    pub mdi: Column,
    /// `ADX`: the Wilder average of `DX = |MDI - PDI| / (MDI + PDI) * 100` over `N` rows.
    pub adx: Column,
    /// `ADXR = (ADX + REF(ADX, MM - 1)) / 2`.
    pub adxr: Column,
}

/// The recursion that produces PDI, MDI and ADX. `ADXR` reads ADX from `adxr_period - 1` rows
/// back, so the state holds a second copy of this stage running that many rows behind the lead:
/// it consumes the same rows through the same recursion, so it yields the lead's earlier ADX bit
/// for bit, with a size independent of the period.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    tr_sum: f64,
    h_sum: f64,
    l_sum: f64,
    mtr: f64,
    dmp: f64,
    dmm: f64,
    dx_sum: f64,
    adx: f64,
}

/// The lead stage and the copy that lags it by `adxr_period - 1` rows.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    lead: Stage,
    lag: Stage,
}

/// What one stage row produces: `(PDI, MDI)` from the first full window and `ADX` from the second.
struct Row {
    di: Option<(f64, f64)>,
    adx: Option<f64>,
}

/// The lead stage reads the previous row. The lagging stage computes the row `adxr_period - 1` back
/// and reads the one before it.
pub(super) fn lookback(_period: usize, adxr_period: usize) -> usize {
    adxr_period.max(1)
}

/// Advances `stage` by the valid row `j`. The first row uses itself as the previous row.
fn stage_step(stage: &mut Stage, w: &Window<'_>, j: usize, period: usize) -> Row {
    let n = period as f64;
    let prev = j.saturating_sub(1);
    let prev_close = w.c(prev);
    let hl = w.h(j) - w.l(j);
    let hcy = (w.h(j) - prev_close).abs();
    let lcy = (prev_close - w.l(j)).abs();
    let hhy = w.h(j) - w.h(prev);
    let lyl = w.l(prev) - w.l(j);
    let tr = hl.max(hcy).max(lcy);
    let h = if hhy > 0.0 && hhy > lyl { hhy } else { 0.0 };
    let l = if lyl > 0.0 && lyl > hhy { lyl } else { 0.0 };
    stage.tr_sum += tr;
    stage.h_sum += h;
    stage.l_sum += l;
    if j + 1 < period {
        return Row {
            di: None,
            adx: None,
        };
    }
    if j + 1 > period {
        stage.mtr = stage.mtr - stage.mtr / n + tr;
        stage.dmp = stage.dmp - stage.dmp / n + h;
        stage.dmm = stage.dmm - stage.dmm / n + l;
    } else {
        stage.mtr = stage.tr_sum;
        stage.dmp = stage.h_sum;
        stage.dmm = stage.l_sum;
    }
    let (pdi, mdi) = if stage.mtr != 0.0 {
        (
            (stage.dmp * 100.0) / stage.mtr,
            (stage.dmm * 100.0) / stage.mtr,
        )
    } else {
        (0.0, 0.0)
    };
    let dx = if mdi + pdi != 0.0 {
        ((mdi - pdi).abs() / (mdi + pdi)) * 100.0
    } else {
        0.0
    };
    stage.dx_sum += dx;
    let adx = if j + 2 >= period * 2 {
        stage.adx = if j + 2 > period * 2 {
            (stage.adx * (n - 1.0) + dx) / n
        } else {
            stage.dx_sum / n
        };
        Some(stage.adx)
    } else {
        None
    };
    Row {
        di: Some((pdi, mdi)),
        adx,
    }
}

/// Advances the formula by the valid row `i`. A zero period leaves every output unset.
pub(super) fn step(
    period: usize,
    adxr_period: usize,
    st: &mut State,
    w: &Window<'_>,
    i: usize,
    out: &mut Out,
) {
    if period == 0 || adxr_period == 0 {
        return;
    }
    let lead = stage_step(&mut st.lead, w, i, period);
    // The lagging copy consumes every row from the first it may read, whether or not ADXR reads it.
    let lagged =
        (i + 1 >= adxr_period).then(|| stage_step(&mut st.lag, w, i + 1 - adxr_period, period));
    if let Some((pdi, mdi)) = lead.di {
        out[0] = Some(pdi);
        out[1] = Some(mdi);
    }
    if let Some(adx) = lead.adx {
        out[2] = Some(adx);
        if i + 3 >= period * 2 + adxr_period {
            let lagged = lagged.and_then(|row| row.adx).unwrap_or(0.0);
            out[3] = Some((lagged + adx) / 2.0);
        }
    }
}

/// KLineChart default: `period = 14`, `adxr_period = 6`.
///
/// `MTR`, `DMP`, and `DMM` start as `N`-row sums and then follow the Wilder recursion
/// `X = X' - X' / N + value`. The first row uses itself as the previous row.
///
/// A fold of the same `step` the chart runtime executes; it assumes finite input.
pub fn dmi(high: &[f64], low: &[f64], close: &[f64], period: usize, adxr_period: usize) -> Dmi {
    let window = Window {
        high,
        low,
        close,
        ..Window::EMPTY
    };
    let [pdi, mdi, adx, adxr] =
        <[Column; 4]>::try_from(fold::<State>(close.len(), 4, |st, i, out| {
            step(period, adxr_period, st, &window, i, out);
        }))
        .expect("dmi folds four columns");
    Dmi {
        pdi,
        mdi,
        adx,
        adxr,
    }
}
