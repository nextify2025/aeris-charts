//! Live-bar easing: an engine-owned display override of a series' drawn last bar.
//!
//! When a streaming tick replaces the drawn last bar in place (same canonical row and time), the
//! displayed high/low/close glide toward the new values with `x += (target - x) * (1 - exp(-dt /
//! tau))`; the open never eases. A brand-new bar, or any other change of the drawn last row,
//! snaps. The canonical columns are never touched: geometry, the last-value line and axis chip,
//! the pulse, chrome bar colors, the crosshair marker on the last bar and the series hit test
//! read the eased values through [`ChartEngine::display_plot`], while every query, snapshot,
//! magnet, autoscale, base value and trading path keeps reading `self.data.plot(id)`.
//!
//! State lives in a [`std::cell::Cell`] on each [`SeriesEntry`] and is written only through shared
//! references, never through `DerefMut for SeriesStore` (which would bump the store revision and
//! force a full scene rebuild per frame). The drawn last row is identified by `(source_row,
//! source_time)` of the series' own canonical columns, so an as-of overlay (whose plot rows
//! repeat canonical rows) and a replay cutoff (whose canonical tail may run past the drawn rows)
//! both key correctly; any mismatch snaps lazily.

use aeris_charts_core::model::plot_list::PlotListView;

use super::*;

/// Upper bound of `live_bar_easing_ms` (the time constant `tau`); `0` is off.
pub const MAX_LIVE_BAR_EASING_MS: f64 = 1000.0;
/// One advance never integrates more than this much wall time, so a host that stopped presenting
/// frames resumes with a visible glide instead of a jump.
pub(crate) const LIVE_BAR_EASING_DT_CAP_MS: f64 = 100.0;
/// The glide settles (snaps exactly to its target) after this many time constants at the latest.
const SETTLE_TAU_MULTIPLE: f64 = 6.0;

/// Per-series easing state (the `Cell` payload).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LiveBarEase {
    /// Canonical row of the drawn last bar this state was keyed to.
    pub(crate) source_row: usize,
    /// The series' own canonical time at `source_row`.
    pub(crate) source_time: i64,
    /// Displayed `[open, high, low, close]` while unsettled.
    pub(crate) displayed: [f64; 4],
    /// `true` once the display equals the canonical row (no override is installed).
    pub(crate) settled: bool,
    /// Clock stamp of the last advance; `None` until the first clock after the transition.
    pub(crate) advance_base_ms: Option<f64>,
    /// Clock stamp of the first advance after the settled → unsettled transition.
    pub(crate) unsettled_at_ms: Option<f64>,
}

impl Default for LiveBarEase {
    fn default() -> Self {
        Self {
            source_row: usize::MAX,
            source_time: i64::MIN,
            displayed: [f64::NAN; 4],
            settled: true,
            advance_base_ms: None,
            unsettled_at_ms: None,
        }
    }
}

impl LiveBarEase {
    fn snapped(drawn: &DrawnLastRow) -> Self {
        Self {
            source_row: drawn.source_row,
            source_time: drawn.source_time,
            displayed: drawn.real,
            settled: true,
            advance_base_ms: None,
            unsettled_at_ms: None,
        }
    }

    fn keyed_to(&self, drawn: &DrawnLastRow) -> bool {
        self.source_row == drawn.source_row && self.source_time == drawn.source_time
    }
}

/// The drawn last row of a series: the canonical row its last plot row shows, that row's own
/// time, and its real values.
struct DrawnLastRow {
    source_row: usize,
    source_time: i64,
    real: [f64; 4],
}

fn finite(values: &[f64; 4]) -> bool {
    values.iter().all(|value| value.is_finite())
}

/// The series' easing time constant in milliseconds, or `None` when easing is off. The JSON
/// option path already rejects negative and non-finite values and clamps to
/// [`MAX_LIVE_BAR_EASING_MS`]; a Rust host writes the `pub` field directly, so the same bound
/// applies here: non-finite or non-positive is off, larger glides with the bound.
fn live_bar_tau(series: &SeriesEntry) -> Option<f64> {
    let tau = series.live_bar_easing_ms;
    (tau.is_finite() && tau > 0.0).then(|| tau.min(MAX_LIVE_BAR_EASING_MS))
}

impl ChartEngine {
    /// `None` when the series has no rows or its last row is hidden by `render_before_time`
    /// (an undrawn bar never glides, so a tick into it keeps the host loop idle).
    fn drawn_last_row(&self, id: SeriesId) -> Option<DrawnLastRow> {
        let plot = self.data.plot(id);
        let row = plot.size().checked_sub(1)?;
        let source_row = plot.source_row(row);
        let (times, columns) = self.data.series_data(id)?;
        let source_time = *times.get(source_row)?;
        if columns.iter().any(|column| column.len() <= source_row) {
            return None;
        }
        if self
            .series_entry(id)?
            .render_before_time
            .is_some_and(|cutoff| source_time >= cutoff)
        {
            return None;
        }
        Some(DrawnLastRow {
            source_row,
            source_time,
            real: [
                columns[0][source_row],
                columns[1][source_row],
                columns[2][source_row],
                columns[3][source_row],
            ],
        })
    }

    /// The easing state after the lazy key check: when the drawn last row is no longer the one the
    /// state was keyed to (a new bar, a pop, a retention trim, a moved replay cutoff) the state
    /// snaps to the real row. `None` when easing is off or the series has no rows.
    fn live_bar_state(&self, series: &SeriesEntry) -> Option<(LiveBarEase, DrawnLastRow)> {
        live_bar_tau(series)?;
        let drawn = self.drawn_last_row(series.id)?;
        let mut state = series.live_bar_ease.get();
        if !state.keyed_to(&drawn) || !finite(&drawn.real) {
            state = LiveBarEase::snapped(&drawn);
            series.live_bar_ease.set(state);
        }
        Some((state, drawn))
    }

    /// The display override for the frame-side readers: the drawn last row's canonical row and
    /// its eased values, only while easing is on, the glide is unsettled and the row still is the
    /// one the glide started on.
    pub(crate) fn live_bar_override(&self, id: SeriesId) -> Option<(usize, [f64; 4])> {
        let series = self.series_entry(id)?;
        let (state, drawn) = self.live_bar_state(series)?;
        (!state.settled).then_some((drawn.source_row, state.displayed))
    }

    /// `self.data.plot(id)` with the eased live bar installed (see [`Self::live_bar_override`]).
    /// Only the frame-side readers use it; queries, snapshots, magnet, autoscale and trading
    /// read the canonical view.
    pub(crate) fn display_plot(&self, id: SeriesId) -> PlotListView<'_> {
        let plot = self.data.plot(id);
        match self.live_bar_override(id) {
            Some((source_row, values)) => plot.with_row_override(source_row, values),
            None => plot,
        }
    }

    /// The Heikin Ashi row a frame-side reader draws for `row` of `plot`: the cached canonical
    /// row unless `plot` overrides that row, in which case the row is recomputed from the eased
    /// raw values and the previous canonical Heikin Ashi row (whitespace rows carry it across).
    /// The cache is never written. `None` when the series is not a Heikin Ashi presentation.
    pub(crate) fn display_heikin_ashi_row(
        &self,
        id: SeriesId,
        plot: PlotListView<'_>,
        row: usize,
    ) -> Option<[f64; 4]> {
        let Some(raw) = plot.overridden_values(row) else {
            return self.heikin_ashi_row(id, row);
        };
        if !self
            .series_entry(id)
            .is_some_and(|series| series.heikin_ashi)
        {
            return None;
        }
        // The previous canonical row is the last one whose projection is finite, exactly the
        // `previous` the cached rebuild carried into this row: whitespace and partially NaN rows
        // both project to NaN and are stepped over. As-of plot rows may repeat the overridden
        // row, so the walk starts at the first plot row showing it.
        let first = plot.row_for_source(plot.source_row(row))?;
        let previous = (0..first)
            .rev()
            .map(|previous| self.heikin_ashi_row(id, previous))
            .find(|values| values.is_none_or(|values| values[3].is_finite()))
            .flatten();
        Some(heikin_ashi::HeikinAshiCache::project(previous, raw))
    }

    /// Writers call this after writing `written_time` to series `id` and synchronizing time
    /// state. Easing off: the state is cleared. The same drawn last row replaced in place: a
    /// settled glide starts (its display keeps the values before the write); an unsettled glide
    /// keeps its clock base and simply follows the moved target, so a 60 Hz feed never freezes,
    /// while its settle window restarts so the glide ends six time constants after the last
    /// change. A write to another row while the drawn row keeps its key (a historical
    /// correction, a replay bar past the cutoff) leaves a running glide alone, since the drawn
    /// row did not change under it, and refreshes a settled display onto the real row. A changed
    /// drawn row, and reduced motion, snap.
    pub(crate) fn note_live_bar_target(&self, id: SeriesId, written_time: i64) {
        let Some(series) = self.series_entry(id) else {
            return;
        };
        let state = series.live_bar_ease.get();
        if live_bar_tau(series).is_none() {
            // Off: writes do not track the display, so a host that turns easing on later by
            // writing the field keys afresh instead of gliding from stale values.
            series.live_bar_ease.set(LiveBarEase::default());
            return;
        }
        let Some(drawn) = self.drawn_last_row(id) else {
            series.live_bar_ease.set(LiveBarEase::default());
            return;
        };
        if !state.keyed_to(&drawn)
            || !finite(&drawn.real)
            || !finite(&state.displayed)
            || self.interaction_options().reduced_motion
        {
            series.live_bar_ease.set(LiveBarEase::snapped(&drawn));
            return;
        }
        if written_time != drawn.source_time {
            if state.settled {
                series.live_bar_ease.set(LiveBarEase::snapped(&drawn));
            }
            return;
        }
        series.live_bar_ease.set(LiveBarEase {
            settled: false,
            advance_base_ms: if state.settled {
                None
            } else {
                state.advance_base_ms
            },
            unsettled_at_ms: None,
            ..state
        });
    }

    /// Display equals the canonical row again: a reinstall, a pop, or an option change. The next
    /// same-time tick glides from the real values, never from stale ones.
    pub(crate) fn snap_live_bar(&self, id: SeriesId) {
        let Some(series) = self.series_entry(id) else {
            return;
        };
        let state = match self.drawn_last_row(id) {
            Some(drawn) if live_bar_tau(series).is_some() => LiveBarEase::snapped(&drawn),
            _ => LiveBarEase::default(),
        };
        series.live_bar_ease.set(state);
    }

    /// Advance every unsettled glide to host clock `now_ms` (milliseconds on any monotonic clock
    /// the host also feeds `input_tick`). The first clock after a settled → unsettled transition
    /// only stamps the base, so that frame still shows the old values; later clocks integrate
    /// `min(now - base, 100 ms)` and settle when every channel is within `max(1e-9, |target| *
    /// 1e-7)` of its target or six time constants have elapsed since the last target change
    /// (a feed that keeps moving the target keeps gliding). A clock below the stamped base (a
    /// host that switched clock source or adapter) starts a new epoch at `now_ms` without
    /// advancing, so a glide can never stay pinned unsettled. Each advance or settle
    /// invalidates that series' geometry, chrome, overlay and axis (never autoscale). Returns
    /// whether any series advanced or settled, which is when the host must present a frame.
    pub fn advance_live_bar_easing(&mut self, now_ms: f64) -> bool {
        if !now_ms.is_finite() {
            return false;
        }
        let mut changed = false;
        for index in 0..self.series.len() {
            let Some((id, advanced)) = self.advance_one_live_bar(index, now_ms) else {
                continue;
            };
            if advanced {
                self.invalidate_frame_series_live_display(id);
                changed = true;
            }
        }
        changed
    }

    fn advance_one_live_bar(&self, index: usize, now_ms: f64) -> Option<(SeriesId, bool)> {
        let series = self.series.get(index)?;
        if series.removed {
            return None;
        }
        let (mut state, drawn) = self.live_bar_state(series)?;
        if state.settled {
            return Some((series.id, false));
        }
        if self.interaction_options().reduced_motion {
            series.live_bar_ease.set(LiveBarEase::snapped(&drawn));
            return Some((series.id, true));
        }
        let Some(base) = state.advance_base_ms else {
            state.advance_base_ms = Some(now_ms);
            state.unsettled_at_ms = Some(now_ms);
            series.live_bar_ease.set(state);
            return Some((series.id, false));
        };
        if now_ms < base {
            // The host clock restarted below the base: a new epoch, nothing to integrate yet.
            state.advance_base_ms = Some(now_ms);
            state.unsettled_at_ms = Some(now_ms);
            series.live_bar_ease.set(state);
            return Some((series.id, false));
        }
        if state.unsettled_at_ms.is_none() {
            // The target moved since the last advance: the settle window restarts here.
            state.unsettled_at_ms = Some(now_ms);
        }
        let dt = (now_ms - base).min(LIVE_BAR_EASING_DT_CAP_MS);
        if dt <= 0.0 {
            series.live_bar_ease.set(state);
            return Some((series.id, false));
        }
        let tau = live_bar_tau(series)?;
        let target = drawn.real;
        let gain = 1.0 - (-dt / tau).exp();
        // The open never eases; high, low and close approach their targets exponentially.
        state.displayed[0] = target[0];
        for (shown, &target) in state.displayed.iter_mut().zip(&target).skip(1) {
            *shown += (target - *shown) * gain;
        }
        state.advance_base_ms = Some(now_ms);
        let within = state
            .displayed
            .iter()
            .zip(&target)
            .skip(1)
            .all(|(shown, &target)| {
                let eps = (target.abs() * 1e-7).max(1e-9);
                (target - shown).abs() <= eps
            });
        let elapsed = state
            .unsettled_at_ms
            .is_some_and(|started| now_ms - started >= SETTLE_TAU_MULTIPLE * tau);
        if within || elapsed {
            state = LiveBarEase::snapped(&drawn);
        }
        series.live_bar_ease.set(state);
        Some((series.id, true))
    }

    /// The host animation clock (milliseconds): advances every live-bar glide, then moves the
    /// pulse clock only when a pulse is drawn or a glide advanced, so a stamp-only tick changes no
    /// overlay key and builds nothing. Browser hosts call this from the rAF loop before `render`;
    /// the GPUI adapter calls it from its prepaint step. Returns whether the step changed what the
    /// chart draws: a glide advanced or settled, or a drawn pulse moved on (a pulse is a change on
    /// every frame).
    pub fn set_animation_time(&mut self, ms: f64) -> bool {
        let eased = self.advance_live_bar_easing(ms);
        let pulsing = self.last_price_pulse_active();
        if pulsing || eased {
            self.animation_time = ms;
        }
        pulsing || eased
    }

    /// Whether any series' live bar is still gliding toward its real values.
    pub fn live_bar_easing_active(&self) -> bool {
        self.series.iter().any(|series| {
            !series.removed
                && self
                    .live_bar_state(series)
                    .is_some_and(|(state, _)| !state.settled)
        })
    }

    /// Whether a host must keep its animation clock running: a last-price pulse is drawn or a live
    /// bar is gliding. The browser package's `wants_animation`.
    pub fn animation_active(&self) -> bool {
        self.last_price_pulse_active() || self.live_bar_easing_active()
    }

    /// Whether a Rust host must request another frame: an input animation (kinetic scroll, held
    /// key) or a live-bar glide is in progress (its first tick only stamps the clock, so the glide
    /// itself asks for the frame that integrates). A drawn pulse keeps frames coming through the
    /// clock step instead: [`Self::set_animation_time`] reports a change on every frame while it
    /// is drawn, and a host requests one more frame after any step that changed chart state.
    pub fn animation_frame_requested(&self) -> bool {
        self.input_animating() || self.live_bar_easing_active()
    }
}
