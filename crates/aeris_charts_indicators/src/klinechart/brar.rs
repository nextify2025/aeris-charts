//! `BRAR` (情绪指标). Ported from KLineChart `src/extension/indicator/brar.ts`.

use super::{empty, Column};

/// BRAR outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Brar {
    /// `BR = SUM(HIGH - REF(CLOSE, 1), N) / SUM(REF(CLOSE, 1) - LOW, N) * 100`, 0 when the divisor
    /// is 0.
    pub br: Column,
    /// `AR = SUM(HIGH - OPEN, N) / SUM(OPEN - LOW, N) * 100`, 0 when the divisor is 0.
    pub ar: Column,
}

/// KLineChart default: `period = 26`. The first row uses its own close as the previous close.
pub fn brar(open: &[f64], high: &[f64], low: &[f64], close: &[f64], period: usize) -> Brar {
    let len = close.len();
    let mut out = Brar {
        br: empty(len),
        ar: empty(len),
    };
    if period == 0 {
        return out;
    }
    let prev_close = |i: usize| close[i.saturating_sub(1)];
    let mut hcy = 0.0;
    let mut cyl = 0.0;
    let mut ho = 0.0;
    let mut ol = 0.0;
    for i in 0..len {
        let pc = prev_close(i);
        ho += high[i] - open[i];
        ol += open[i] - low[i];
        hcy += high[i] - pc;
        cyl += pc - low[i];
        if i + 1 >= period {
            out.ar[i] = Some(if ol != 0.0 { (ho / ol) * 100.0 } else { 0.0 });
            out.br[i] = Some(if cyl != 0.0 { (hcy / cyl) * 100.0 } else { 0.0 });
            let ago = i + 1 - period;
            // KLineChart reads `dataList[i - N] ?? dataList[i - (N - 1)]`.
            let ago_prev_close = if i >= period {
                close[i - period]
            } else {
                close[ago]
            };
            hcy -= high[ago] - ago_prev_close;
            cyl -= ago_prev_close - low[ago];
            ho -= high[ago] - open[ago];
            ol -= open[ago] - low[ago];
        }
    }
    out
}
