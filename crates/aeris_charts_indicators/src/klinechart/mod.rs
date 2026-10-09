//! Technical indicators ported from [KLineChart](https://github.com/klinecharts/KLineChart)
//! v10.0.3 (`src/extension/indicator`).
//!
//! These keep KLineChart's formulas, warm-up rules, and edge-case handling so every value matches
//! that library bar for bar; `tests/klinechart_parity.rs` checks all of them against KLineChart's
//! own `calc` output. They follow the conventions of mainland-China and Hong Kong charting software,
//! which differ from the crate-root formulas in several places:
//!
//! - the MACD histogram is `(DIF - DEA) * 2`, and EMAs are seeded with the simple average of their
//!   first `N` inputs;
//! - [`sma`] is the weighted `SMA(X, N, M)` smoothing, while the plain rolling mean is [`ma`];
//! - KDJ starts its `K` and `D` recursions from 50.
//!
//! Every function takes only the input columns it reads and returns one [`Column`] per KLineChart
//! output figure, in KLineChart's figure order. `None` marks a row where KLineChart leaves the
//! figure unset. KLineChart does not validate periods; here a zero period yields an all-`None`
//! output instead of dividing by zero.
//!
//! All 27 formulas are row steppers: each is one `State` (`Copy`, scalars and fixed-size arrays
//! only) and one `step` over a single non-whitespace row. The public functions are folds of those
//! steps, over columns the caller guarantees finite; they do no whitespace handling. The chart
//! runtime ([`crate::IncrementalState::klinechart`]) replays the same steps from checkpointed
//! state one row at a time and skips whitespace rows in place (NaN output, no state change), so
//! each row that carries a sample gets the value the whole-series function gives that row of the
//! series with the whitespace rows removed (see `stepper`).
//!
//! [`Indicator`] bundles a template with its parameters. It is the form a chart binds
//! ([`crate::IncrementalState::klinechart`]) and persists, and it carries the presentation
//! metadata a host needs to draw the outputs the way KLineChart does.
//!
//! KLineChart is Copyright (c) 2019 lihu and licensed under the Apache License, Version 2.0. Each
//! module names the KLineChart source file it was translated from. Changes: translated from
//! TypeScript to Rust, and outputs are returned as columns instead of per-row objects.

mod average_price;
mod awesome_oscillator;
mod bias;
mod bollinger_bands;
mod brar;
mod bull_and_bear_index;
mod commodity_channel_index;
mod current_ratio;
mod different_of_moving_average;
mod directional_movement_index;
mod ease_of_movement_value;
mod exponential_moving_average;
mod indicator;
mod momentum;
mod moving_average;
mod moving_average_convergence_divergence;
mod on_balance_volume;
mod price_and_volume_trend;
mod psychological_line;
mod rate_of_change;
mod relative_strength_index;
mod simple_moving_average;
mod stepper;
mod stoch;
mod stop_and_reverse;
mod triple_exponentially_smoothed_average;
mod volume;
mod volume_ratio;
mod williams_r;

pub(crate) use stepper::KlRuntime;

pub use average_price::avp;
pub use awesome_oscillator::ao;
pub use bias::bias;
pub use bollinger_bands::{Boll, boll};
pub use brar::{Brar, brar};
pub use bull_and_bear_index::bbi;
pub use commodity_channel_index::cci;
pub use current_ratio::{Cr, cr};
pub use different_of_moving_average::{Dma, dma};
pub use directional_movement_index::{Dmi, dmi};
pub use ease_of_movement_value::{Emv, emv};
pub use exponential_moving_average::ema;
pub use indicator::{Bars, Figure, Indicator, MAX_PERIOD, NAMES, Param, Placement, ValueFormat};
pub use momentum::{Mtm, mtm};
pub use moving_average::ma;
pub use moving_average_convergence_divergence::{Macd, macd};
pub use on_balance_volume::{Obv, obv};
pub use price_and_volume_trend::pvt;
pub use psychological_line::{Psy, psy};
pub use rate_of_change::{Roc, roc};
pub use relative_strength_index::rsi;
pub use simple_moving_average::sma;
pub use stoch::{Kdj, kdj};
pub use stop_and_reverse::sar;
pub use triple_exponentially_smoothed_average::{Trix, trix};
pub use volume::vol;
pub use volume_ratio::{Vr, vr};
pub use williams_r::wr;

/// One indicator output figure: a value per input row, `None` where KLineChart leaves it unset.
pub type Column = Vec<Option<f64>>;

/// KLineChart's `getMaxMin(slice, 'high', 'low')`: the highest high and lowest low of a window.
fn highest_high_lowest_low(high: &[f64], low: &[f64]) -> (f64, f64) {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let mut highest = -MAX_SAFE_INTEGER;
    let mut lowest = MAX_SAFE_INTEGER;
    for (&h, &l) in high.iter().zip(low) {
        highest = highest.max(h);
        lowest = lowest.min(l);
    }
    (highest, lowest)
}
