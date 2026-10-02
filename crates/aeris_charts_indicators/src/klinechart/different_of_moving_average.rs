//! `DMA` (平行线差). Ported from KLineChart
//! `src/extension/indicator/differentOfMovingAverage.ts`.

use super::{empty, rolling_mean, Column};

/// DMA outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Dma {
    /// `DIF = MA(CLOSE, N1) - MA(CLOSE, N2)`.
    pub dma: Column,
    /// `AMA = MA(DIF, M)`.
    pub ama: Column,
}

/// KLineChart default: `short = 10`, `long = 50`, `signal = 10`.
pub fn dma(close: &[f64], short: usize, long: usize, signal: usize) -> Dma {
    let len = close.len();
    let mut out = Dma {
        dma: empty(len),
        ama: empty(len),
    };
    if short == 0 || long == 0 || signal == 0 {
        return out;
    }
    let ma_short = rolling_mean(close, short);
    let ma_long = rolling_mean(close, long);
    let max_period = short.max(long);
    let m = signal as f64;
    let mut dma_sum = 0.0;
    for i in max_period - 1..len {
        let dif = ma_short[i].unwrap_or(0.0) - ma_long[i].unwrap_or(0.0);
        out.dma[i] = Some(dif);
        dma_sum += dif;
        if i + 2 >= max_period + signal {
            out.ama[i] = Some(dma_sum / m);
            dma_sum -= out.dma[i + 1 - signal].unwrap_or(0.0);
        }
    }
    out
}
