//! `BOLL` (布林线). Ported from KLineChart `src/extension/indicator/bollingerBands.ts`.

use super::{empty, Column};

/// Bollinger bands in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Boll {
    /// `MID + K * MD`.
    pub up: Column,
    /// `MID`: the `N`-row rolling mean of the close.
    pub mid: Column,
    /// `MID - K * MD`.
    pub dn: Column,
}

/// KLineChart default: `period = 20`, `multiplier = 2`. `MD` is the population standard deviation
/// of the window's closes around `MID`.
pub fn boll(close: &[f64], period: usize, multiplier: f64) -> Boll {
    let len = close.len();
    let mut out = Boll {
        up: empty(len),
        mid: empty(len),
        dn: empty(len),
    };
    if period == 0 {
        return out;
    }
    let n = period as f64;
    let mut close_sum = 0.0;
    for (i, &c) in close.iter().enumerate() {
        close_sum += c;
        if i + 1 >= period {
            let start = i + 1 - period;
            let mid = close_sum / n;
            let md = deviation(&close[start..=i], mid);
            out.mid[i] = Some(mid);
            out.up[i] = Some(mid + multiplier * md);
            out.dn[i] = Some(mid - multiplier * md);
            close_sum -= close[start];
        }
    }
    out
}

/// KLineChart's `getBollMd`.
fn deviation(window: &[f64], mean: f64) -> f64 {
    let mut sum = 0.0;
    for &c in window {
        let diff = c - mean;
        sum += diff * diff;
    }
    (sum.abs() / window.len() as f64).sqrt()
}
