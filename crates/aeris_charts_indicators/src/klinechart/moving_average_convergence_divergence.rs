//! `MACD`. Ported from KLineChart
//! `src/extension/indicator/movingAverageConvergenceDivergence.ts`.

use super::{empty, exponential_moving_average::seeded_ema, Column};

/// MACD outputs in KLineChart figure order.
#[derive(Clone, Debug, PartialEq)]
pub struct Macd {
    /// `DIF = EMA(CLOSE, SHORT) - EMA(CLOSE, LONG)`.
    pub dif: Column,
    /// `DEA`: the EMA of `DIF` over `SIGNAL` rows, seeded with the average of the first `SIGNAL`
    /// DIF values.
    pub dea: Column,
    /// The histogram, `(DIF - DEA) * 2` (the Chinese-software convention).
    pub macd: Column,
}

/// KLineChart default: `short = 12`, `long = 26`, `signal = 9`.
pub fn macd(close: &[f64], short: usize, long: usize, signal: usize) -> Macd {
    let len = close.len();
    let mut out = Macd {
        dif: empty(len),
        dea: empty(len),
        macd: empty(len),
    };
    if short == 0 || long == 0 || signal == 0 {
        return out;
    }
    let ema_short = seeded_ema(close, short);
    let ema_long = seeded_ema(close, long);
    let max_period = short.max(long);
    let m = signal as f64;
    let mut dif_sum = 0.0;
    let mut dea = 0.0;
    for i in max_period - 1..len {
        let (Some(fast), Some(slow)) = (ema_short[i], ema_long[i]) else {
            continue;
        };
        let dif = fast - slow;
        out.dif[i] = Some(dif);
        dif_sum += dif;
        if i + 2 >= max_period + signal {
            dea = if i + 2 > max_period + signal {
                (dif * 2.0 + dea * (m - 1.0)) / (m + 1.0)
            } else {
                dif_sum / m
            };
            out.macd[i] = Some((dif - dea) * 2.0);
            out.dea[i] = Some(dea);
        }
    }
    out
}
