//! `KDJ` (随机指标). Ported from KLineChart `src/extension/indicator/stoch.ts`.

use super::{empty, highest_high_lowest_low, Column};

/// KDJ outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Kdj {
    /// `K = ((M1 - 1) * K' + RSV) / M1`, starting from `K' = 50`.
    pub k: Column,
    /// `D = ((M2 - 1) * D' + K) / M2`, starting from `D' = 50`.
    pub d: Column,
    /// `J = 3K - 2D`.
    pub j: Column,
}

/// KLineChart default: `period = 9`, `k_smoothing = 3`, `d_smoothing = 3`.
///
/// `RSV = (CLOSE - LLV(LOW, N)) / (HHV(HIGH, N) - LLV(LOW, N)) * 100`, with the range replaced by 1
/// when it is zero.
pub fn kdj(
    high: &[f64],
    low: &[f64],
    close: &[f64],
    period: usize,
    k_smoothing: usize,
    d_smoothing: usize,
) -> Kdj {
    let len = close.len();
    let mut out = Kdj {
        k: empty(len),
        d: empty(len),
        j: empty(len),
    };
    if period == 0 || k_smoothing == 0 || d_smoothing == 0 {
        return out;
    }
    let m1 = k_smoothing as f64;
    let m2 = d_smoothing as f64;
    for i in period - 1..len {
        let start = i + 1 - period;
        let (hn, ln) = highest_high_lowest_low(&high[start..=i], &low[start..=i]);
        let range = hn - ln;
        let rsv = ((close[i] - ln) / if range == 0.0 { 1.0 } else { range }) * 100.0;
        let prev_k = i.checked_sub(1).and_then(|p| out.k[p]).unwrap_or(50.0);
        let prev_d = i.checked_sub(1).and_then(|p| out.d[p]).unwrap_or(50.0);
        let k = ((m1 - 1.0) * prev_k + rsv) / m1;
        let d = ((m2 - 1.0) * prev_d + k) / m2;
        out.k[i] = Some(k);
        out.d[i] = Some(d);
        out.j[i] = Some(3.0 * k - 2.0 * d);
    }
    out
}
