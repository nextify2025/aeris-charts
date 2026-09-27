//! Price scale coordinate math and interactions. Port of `src/model/price-scale.ts`
//! (data-source management and formatter selection live at a higher layer).

use crate::model::price_range::PriceRange;
use crate::scale::log_formula::{self, LogFormula, DEF_LOG_FORMULA};
use crate::scale::price_tick_span_calculator::{align_span_to_min_move, composite_tick_span};
use crate::Coordinate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceScaleMode {
    Normal,
    Logarithmic,
    Percentage,
    IndexedTo100,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceScaleMargins {
    /// 0..1 fraction of height.
    pub top: f64,
    pub bottom: f64,
}

#[derive(Clone, Debug)]
pub struct PriceScaleCoreOptions {
    pub mode: PriceScaleMode,
    pub invert_scale: bool,
    pub auto_scale: bool,
    pub scale_margins: PriceScaleMargins,
    /// Tick mark label density (default 2.5); higher = fewer marks.
    pub tick_mark_density: f64,
    /// Axis tick label size in px (used for tick mark height). Engine-synced from the
    /// resolved axis metrics whenever layout font settings change; not host-configurable
    /// (no patch key, not serialized).
    pub font_size: f64,
    /// reference `alignLabels` (default true): push the axis' boxed labels apart so they cannot
    /// overlap each other or leave the pane edge.
    pub align_labels: bool,
    /// reference `ticksVisible` (default false): draw a small tick mark beside each axis label.
    pub ticks_visible: bool,
    /// reference `entireTextOnly` (default false): skip top/bottom tick marks whose label text
    /// would be clipped by the pane edge.
    pub entire_text_only: bool,
    /// reference `minimumWidth` (default 0): floor for the axis strip width; the measured width
    /// still wins when the labels need more room.
    pub minimum_width: f64,
    /// reference `textColor` (default `None` = follow `layout.textColor`). Stored verbatim as a
    /// CSS string; parsed at render time.
    pub text_color: Option<String>,
    /// Aeris extension (industry-standard, default true): draw round-figure tick labels in the
    /// bold font — multiples of `step × 10` on uniform ticks, exact powers of ten on
    /// non-uniform (log) ticks.
    pub bold_round_labels: bool,
    /// reference `ensureEdgeTickMarksVisible` (default false): while autoscaled, reserve half a
    /// font height at both edges and place a rounded tick mark at the very top and bottom.
    pub ensure_edge_tick_marks_visible: bool,
    /// Aeris extension (default `None`): explicit base price for the percentage and
    /// indexed-to-100 modes, e.g. the previous close. When set, every source on this scale and
    /// every drawing bound to it converts against this value instead of its first visible bar or
    /// the chart comparison anchor.
    pub base_value: Option<f64>,
    /// Aeris extension (default `None`): center the autoscaled range on this raw price:
    /// `center ± max|price − center|`, before scale margins apply.
    pub autoscale_center: Option<f64>,
    /// Aeris extension (default false): stable autoscale. The range expands at once when visible
    /// data exceeds it but shrinks only when the data one bar beyond the visible edges leaves
    /// more than [`STABLE_AUTOSCALE_SHRINK_FRACTION`] of the range unused, so sub-bar pans and
    /// kinetic coasts cannot flip the range back and forth. `false` keeps the reference's exact
    /// per-frame range.
    pub stable_auto_scale: bool,
}

/// Unused fraction of a stable autoscale range that triggers a shrink to the data.
pub const STABLE_AUTOSCALE_SHRINK_FRACTION: f64 = 0.2;

impl Default for PriceScaleCoreOptions {
    fn default() -> Self {
        Self {
            mode: PriceScaleMode::Normal,
            invert_scale: false,
            auto_scale: true,
            scale_margins: PriceScaleMargins {
                top: 0.2,
                bottom: 0.1,
            },
            tick_mark_density: 2.5,
            font_size: 12.0,
            // reference defaults (price-scale-options-defaults.ts).
            align_labels: true,
            ticks_visible: false,
            entire_text_only: false,
            minimum_width: 0.0,
            text_color: None,
            bold_round_labels: true,
            ensure_edge_tick_marks_visible: false,
            base_value: None,
            autoscale_center: None,
            stable_auto_scale: false,
        }
    }
}

/// A generated tick mark: coordinate (media px) + the logical value it represents.
/// Label formatting is the caller's concern (price formatter / localization).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceMark {
    pub coord: Coordinate,
    pub logical: f64,
    /// A boundary mark added by `ensure_edge_tick_marks_visible`; it sits off the uniform span.
    pub edge: bool,
}

#[derive(Clone, Debug)]
pub struct PriceScaleCore {
    options: PriceScaleCoreOptions,
    height: f64,
    price_range: Option<PriceRange>,
    /// Extra px margins requested by autoscale info providers.
    margin_above: f64,
    margin_below: f64,
    /// Offset of the owning pane in chart-content space. The scale's geometry is entirely
    /// pane-local; this is the single explicit transform into the host's chart coordinates.
    pane_offset: f64,
    log_formula: LogFormula,
    min_move: f64,
    scale_start_point: Option<f64>,
    scroll_start_point: Option<f64>,
    price_range_snapshot: Option<PriceRange>,
    /// Whether `price_range` is a stable-autoscale result the next pass may keep or grow.
    /// Cleared by mode/base/center changes, explicit autoscale resets, and data replacement.
    stable_range_valid: bool,
    /// Canonical source revision for coordinate and axis-presentation state.
    revision: u64,
}

impl PriceScaleCore {
    pub fn new(options: PriceScaleCoreOptions) -> Self {
        Self {
            options,
            height: 0.0,
            price_range: None,
            margin_above: 0.0,
            margin_below: 0.0,
            pane_offset: 0.0,
            log_formula: DEF_LOG_FORMULA,
            min_move: 0.01,
            scale_start_point: None,
            scroll_start_point: None,
            price_range_snapshot: None,
            stable_range_valid: false,
            revision: 1,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }

    pub fn options(&self) -> &PriceScaleCoreOptions {
        &self.options
    }

    pub fn mode(&self) -> PriceScaleMode {
        self.options.mode
    }

    /// Change coordinate mode while keeping any convertible manual range coherent. Percentage and
    /// indexed modes require a series base value, so—as in reference—they always re-enter autoscale.
    pub fn set_mode(&mut self, mode: PriceScaleMode) {
        let old = self.options.mode;
        if old == mode {
            return;
        }
        let raw_range = match (old, self.price_range) {
            (_, None) => None,
            (PriceScaleMode::Logarithmic, Some(range)) => Some(
                log_formula::convert_price_range_from_log(&range, &self.log_formula),
            ),
            (PriceScaleMode::Normal, Some(range)) => Some(range),
            // Percentage/indexed ranges cannot be reversed without the source's base value.
            _ => None,
        };
        self.options.mode = mode;
        self.stable_range_valid = false;
        match mode {
            PriceScaleMode::Normal => self.price_range = raw_range,
            PriceScaleMode::Logarithmic => {
                self.log_formula = log_formula::log_formula_for_price_range(raw_range.as_ref());
                self.price_range = raw_range
                    .as_ref()
                    .map(|range| log_formula::convert_price_range_to_log(range, &self.log_formula));
            }
            PriceScaleMode::Percentage | PriceScaleMode::IndexedTo100 => {
                self.options.auto_scale = true;
                self.price_range = None;
            }
        }
        self.changed();
    }

    pub fn is_log(&self) -> bool {
        self.options.mode == PriceScaleMode::Logarithmic
    }

    pub fn is_percentage(&self) -> bool {
        self.options.mode == PriceScaleMode::Percentage
    }

    pub fn is_indexed_to_100(&self) -> bool {
        self.options.mode == PriceScaleMode::IndexedTo100
    }

    pub fn is_inverted(&self) -> bool {
        self.options.invert_scale
    }

    pub fn set_invert_scale(&mut self, inverted: bool) {
        if self.options.invert_scale != inverted {
            self.options.invert_scale = inverted;
            self.changed();
        }
    }

    /// reference `alignLabels` — gate the axis' label overlap resolution.
    pub fn set_align_labels(&mut self, align: bool) {
        if self.options.align_labels != align {
            self.options.align_labels = align;
            self.changed();
        }
    }

    /// reference `ticksVisible` — draw small tick marks beside the axis labels.
    pub fn set_ticks_visible(&mut self, visible: bool) {
        if self.options.ticks_visible != visible {
            self.options.ticks_visible = visible;
            self.changed();
        }
    }

    /// reference `entireTextOnly` — skip corner tick marks whose label would be clipped.
    pub fn set_entire_text_only(&mut self, entire: bool) {
        if self.options.entire_text_only != entire {
            self.options.entire_text_only = entire;
            self.changed();
        }
    }

    /// Engine-synced axis tick label size in px (finite, positive). Drives tick mark
    /// height together with `tick_mark_density`; the engine owns this value (see
    /// `ChartEngine::sync_axis_tick_fonts`) and invalidates on every sync path.
    pub fn set_tick_font_size(&mut self, size: f64) {
        if size.is_finite() && size > 0.0 && self.options.font_size != size {
            self.options.font_size = size;
            self.changed();
        }
    }

    /// reference `minimumWidth` — floor for the axis strip width (non-negative, finite).
    pub fn set_minimum_width(&mut self, width: f64) {
        if width.is_finite() && width >= 0.0 && self.options.minimum_width != width {
            self.options.minimum_width = width;
            self.changed();
        }
    }

    /// reference `textColor` — `None` follows `layout.textColor`; stored verbatim, parsed at
    /// render time.
    pub fn set_text_color(&mut self, css: Option<String>) {
        if self.options.text_color != css {
            self.options.text_color = css;
            self.changed();
        }
    }

    /// Aeris extension (industry-standard, default true): bold round-figure tick labels.
    pub fn set_bold_round_labels(&mut self, bold: bool) {
        if self.options.bold_round_labels != bold {
            self.options.bold_round_labels = bold;
            self.changed();
        }
    }

    /// reference `tickMarkDensity` (default 2.5): tick label spacing in font heights. Finite
    /// positive values only; higher values produce fewer marks.
    pub fn set_tick_mark_density(&mut self, density: f64) -> bool {
        if !density.is_finite() || density <= 0.0 {
            return false;
        }
        if self.options.tick_mark_density != density {
            self.options.tick_mark_density = density;
            self.changed();
        }
        true
    }

    /// reference `ensureEdgeTickMarksVisible`: boundary tick marks plus half-font edge padding
    /// while the scale autoscales.
    pub fn set_ensure_edge_tick_marks_visible(&mut self, visible: bool) {
        if self.options.ensure_edge_tick_marks_visible != visible {
            self.options.ensure_edge_tick_marks_visible = visible;
            self.changed();
        }
    }

    /// Explicit percentage/indexed-to-100 base price (`None` restores the first-visible/anchor
    /// base). Rejects non-finite and zero bases, which cannot produce finite coordinates.
    pub fn set_base_value(&mut self, base: Option<f64>) -> bool {
        if base.is_some_and(|value| !value.is_finite() || value == 0.0) {
            return false;
        }
        if self.options.base_value != base {
            self.options.base_value = base;
            self.stable_range_valid = false;
            self.changed();
        }
        true
    }

    /// Raw price the autoscaled range is centered on (`None` restores the plain data range).
    pub fn set_autoscale_center(&mut self, center: Option<f64>) -> bool {
        if center.is_some_and(|value| !value.is_finite()) {
            return false;
        }
        if self.options.autoscale_center != center {
            self.options.autoscale_center = center;
            self.stable_range_valid = false;
            self.changed();
        }
        true
    }

    /// Opt into (or out of) the stable autoscale range. Either transition restarts from the exact
    /// data range on the next autoscale pass.
    pub fn set_stable_auto_scale(&mut self, stable: bool) {
        if self.options.stable_auto_scale != stable {
            self.options.stable_auto_scale = stable;
            self.stable_range_valid = false;
            self.changed();
        }
    }

    /// Forget the retained stable-autoscale range so the next pass starts from the exact data
    /// range (series data replacement, explicit resets).
    pub fn reset_autoscale_stabilization(&mut self) {
        self.stable_range_valid = false;
    }

    /// Restore only axis presentation fields to their canonical defaults. Scale mode, inversion,
    /// autoscale/manual range, margins, label-layout behavior, and gesture state are preserved.
    pub fn reset_style_to_defaults(&mut self) {
        let defaults = PriceScaleCoreOptions::default();
        self.set_ticks_visible(defaults.ticks_visible);
        self.set_text_color(defaults.text_color);
        self.set_bold_round_labels(defaults.bold_round_labels);
    }

    pub fn is_auto_scale(&self) -> bool {
        self.options.auto_scale
    }

    pub fn set_auto_scale(&mut self, v: bool) {
        if v {
            // Re-enabling (or reaffirming) autoscale cancels any stale manual gesture snapshot.
            // A later pointer session must start from the newly autoscaled range.
            self.scale_start_point = None;
            self.scroll_start_point = None;
            self.price_range_snapshot = None;
            // An explicit autoscale (re)enable is a reset: stable mode restarts from exact data.
            self.stable_range_valid = false;
        }
        if self.options.auto_scale != v {
            self.options.auto_scale = v;
            self.changed();
        }
    }

    pub fn log_formula(&self) -> &LogFormula {
        &self.log_formula
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    pub fn set_height(&mut self, height: f64) {
        if self.height != height {
            self.height = height;
            self.changed();
        }
    }

    /// Offset of the owning pane's top edge in chart-content space.
    pub fn pane_offset(&self) -> f64 {
        self.pane_offset
    }

    /// Place this pane-local scale inside the chart content area. Every coordinate the scale
    /// produces or consumes on its public API is chart-content space; everything in between —
    /// height, margins, internal height, tick marks, gestures — is resolved against the pane's
    /// own slot alone, so panes never share axis geometry.
    pub fn set_pane_offset(&mut self, offset: f64) {
        if self.pane_offset != offset {
            self.pane_offset = offset;
            self.changed();
        }
    }

    pub fn price_range(&self) -> Option<&PriceRange> {
        self.price_range.as_ref()
    }

    /// Price-scale API representation. Logarithmic storage is converted back to raw prices; the
    /// other modes expose their current display-domain range directly, matching reference.
    pub fn price_range_for_api(&self) -> Option<PriceRange> {
        let range = self.price_range?;
        Some(if self.is_log() {
            let raw = log_formula::convert_price_range_from_log(&range, &self.log_formula);
            let precision = if self.min_move > 0.0 {
                (-self.min_move.log10()).ceil().clamp(0.0, 15.0) as i32
            } else {
                0
            };
            let factor = 10_f64.powi(precision);
            let rounded = |value: f64| {
                (((value / self.min_move).round() * self.min_move) * factor).round() / factor
            };
            PriceRange::new(rounded(raw.min_value()), rounded(raw.max_value()))
        } else {
            range
        })
    }

    pub fn price_range_from_api(&self, range: &PriceRange) -> PriceRange {
        if self.is_log() {
            log_formula::convert_price_range_to_log(range, &self.log_formula)
        } else {
            *range
        }
    }

    pub fn set_price_range(&mut self, range: Option<PriceRange>) {
        if self.price_range != range {
            self.price_range = range;
            self.changed();
        }
    }

    pub fn set_internal_margins(&mut self, above_px: f64, below_px: f64) {
        if (self.margin_above, self.margin_below) != (above_px, below_px) {
            self.margin_above = above_px;
            self.margin_below = below_px;
            self.changed();
        }
    }

    /// Set the fractional scale margins (`top`/`bottom` as fractions of scale height). Used to pin
    /// an overlay scale to a band of the pane (e.g. volume in the bottom fifth).
    pub fn set_scale_margins(&mut self, top: f64, bottom: f64) {
        if (
            self.options.scale_margins.top,
            self.options.scale_margins.bottom,
        ) != (top, bottom)
        {
            self.options.scale_margins.top = top;
            self.options.scale_margins.bottom = bottom;
            self.changed();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.height == 0.0
            || self.price_range.is_none()
            || self.price_range.as_ref().is_some_and(|r| r.is_empty())
    }

    /// reference `hasVisibleEdgeMarks`: edge tick marks only apply while autoscaled.
    fn has_visible_edge_marks(&self) -> bool {
        self.options.ensure_edge_tick_marks_visible && self.options.auto_scale
    }

    /// reference `getEdgeMarksPadding`: half the tick font height.
    fn edge_marks_padding(&self) -> f64 {
        self.options.font_size / 2.0
    }

    /// Pixel margins from autoscale providers, widened to the edge-mark padding while edge marks
    /// are visible (reference `_recalculatePriceRangeImpl`).
    fn internal_margins_px(&self) -> (f64, f64) {
        if self.has_visible_edge_marks() {
            let padding = self.edge_marks_padding();
            (
                self.margin_above.max(padding),
                self.margin_below.max(padding),
            )
        } else {
            (self.margin_above, self.margin_below)
        }
    }

    fn top_margin_px(&self) -> f64 {
        let (above, below) = self.internal_margins_px();
        if self.is_inverted() {
            self.options.scale_margins.bottom * self.height + below
        } else {
            self.options.scale_margins.top * self.height + above
        }
    }

    fn bottom_margin_px(&self) -> f64 {
        let (above, below) = self.internal_margins_px();
        if self.is_inverted() {
            self.options.scale_margins.top * self.height + above
        } else {
            self.options.scale_margins.bottom * self.height + below
        }
    }

    pub fn internal_height(&self) -> f64 {
        self.height - self.top_margin_px() - self.bottom_margin_px()
    }

    fn inverted_coordinate(&self, coordinate: f64) -> f64 {
        if self.is_inverted() {
            coordinate
        } else {
            self.height - 1.0 - coordinate
        }
    }

    // --- mode transforms ---

    /// price -> logical (space in which the range is linear).
    fn price_to_logical(&self, price: f64, base_value: f64) -> f64 {
        match self.options.mode {
            PriceScaleMode::Normal => price,
            PriceScaleMode::Logarithmic => {
                if price != 0.0 {
                    log_formula::to_log(price, &self.log_formula)
                } else {
                    price
                }
            }
            PriceScaleMode::Percentage => log_formula::to_percent(price, base_value),
            PriceScaleMode::IndexedTo100 => log_formula::to_indexed_to_100(price, base_value),
        }
    }

    /// Convert a raw source range to this scale's logical coordinate domain using the source's
    /// first visible value. Returns `None` when the mode/base cannot produce finite coordinates.
    pub fn price_range_to_logical(&self, raw: &PriceRange, base_value: f64) -> Option<PriceRange> {
        let a = self.price_to_logical(raw.min_value(), base_value);
        let b = self.price_to_logical(raw.max_value(), base_value);
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        Some(PriceRange::new(a.min(b), a.max(b)))
    }

    /// Public scalar counterpart used by engine-owned labels and series API conversion.
    pub fn price_to_logical_value(&self, price: f64, base_value: f64) -> f64 {
        self.price_to_logical(price, base_value)
    }

    fn logical_to_price(&self, logical: f64, base_value: f64) -> f64 {
        match self.options.mode {
            PriceScaleMode::Normal => logical,
            PriceScaleMode::Logarithmic => log_formula::from_log(logical, &self.log_formula),
            PriceScaleMode::Percentage => log_formula::from_percent(logical, base_value),
            PriceScaleMode::IndexedTo100 => log_formula::from_indexed_to_100(logical, base_value),
        }
    }

    // --- coordinate conversion ---
    //
    // Note on log mode: the stored price range is already in log space, so
    // `logical_to_coordinate` applies `to_log` to its input (matching reference where
    // `_logicalToCoordinate` re-transforms), while percent/indexed inputs are pre-transformed.

    pub fn logical_to_coordinate(&self, logical: f64) -> Coordinate {
        self.logical_to_coordinate_local(logical) + self.pane_offset
    }

    /// Pane-local coordinate (0 = the pane's top edge), before the pane-offset transform.
    fn logical_to_coordinate_local(&self, logical: f64) -> Coordinate {
        if self.is_empty() {
            return 0.0;
        }

        let logical = if self.is_log() && logical != 0.0 {
            log_formula::to_log(logical, &self.log_formula)
        } else {
            logical
        };

        let Some(range) = self.price_range.as_ref() else {
            return 0.0;
        };
        let inv_coordinate = self.bottom_margin_px()
            + (self.internal_height() - 1.0) * (logical - range.min_value()) / range.length();
        self.inverted_coordinate(inv_coordinate)
    }

    pub fn coordinate_to_logical(&self, coordinate: f64) -> f64 {
        self.coordinate_to_logical_local(coordinate - self.pane_offset)
    }

    fn coordinate_to_logical_local(&self, coordinate: f64) -> f64 {
        if self.is_empty() {
            return 0.0;
        }

        let inv_coordinate = self.inverted_coordinate(coordinate);
        let Some(range) = self.price_range.as_ref() else {
            return 0.0;
        };
        let logical = range.min_value()
            + range.length()
                * ((inv_coordinate - self.bottom_margin_px()) / (self.internal_height() - 1.0));

        if self.is_log() {
            log_formula::from_log(logical, &self.log_formula)
        } else {
            logical
        }
    }

    pub fn price_to_coordinate(&self, price: f64, base_value: f64) -> Coordinate {
        let logical = match self.options.mode {
            PriceScaleMode::Percentage => log_formula::to_percent(price, base_value),
            PriceScaleMode::IndexedTo100 => log_formula::to_indexed_to_100(price, base_value),
            _ => price,
        };
        self.logical_to_coordinate(logical)
    }

    pub fn coordinate_to_price(&self, coordinate: f64, base_value: f64) -> f64 {
        let logical = self.coordinate_to_logical(coordinate);
        match self.options.mode {
            PriceScaleMode::Percentage => log_formula::from_percent(logical, base_value),
            PriceScaleMode::IndexedTo100 => log_formula::from_indexed_to_100(logical, base_value),
            // log handled inside coordinate_to_logical; normal is identity
            _ => logical,
        }
    }

    /// Batch conversion for OHLC bars — hot path. `base_value` is the series' first value.
    /// Writes 4 y-coordinates per bar.
    #[allow(clippy::too_many_arguments)]
    pub fn bar_prices_to_coordinates(
        &self,
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
        base_value: f64,
        out: &mut [[f64; 4]],
    ) {
        if self.is_empty() {
            return;
        }
        let bh = self.bottom_margin_px();
        let Some(range) = self.price_range.as_ref() else {
            return;
        };
        let min = range.min_value();
        let max = range.max_value();
        let ih = self.internal_height() - 1.0;
        let is_inverted = self.is_inverted();
        let pane_offset = self.pane_offset;
        let hmm = ih / (max - min);
        let needs_transform = self.options.mode != PriceScaleMode::Normal;

        for i in 0..out.len() {
            let mut prices = [open[i], high[i], low[i], close[i]];
            if needs_transform {
                for p in &mut prices {
                    *p = self.price_to_logical(*p, base_value);
                }
            }
            for (j, p) in prices.iter().enumerate() {
                let inv_coordinate = bh + hmm * (p - min);
                out[i][j] = pane_offset
                    + if is_inverted {
                        inv_coordinate
                    } else {
                        self.height - 1.0 - inv_coordinate
                    };
            }
        }
    }

    // --- axis-drag scale ---

    pub fn start_scale(&mut self, x: f64) {
        if self.is_percentage() || self.is_indexed_to_100() {
            return;
        }
        if self.scale_start_point.is_some() || self.price_range_snapshot.is_some() {
            return;
        }
        if self.is_empty() {
            return;
        }

        // invert x (pane-local)
        self.scale_start_point = Some(self.height - (x - self.pane_offset));
        self.price_range_snapshot = self.price_range;
    }

    pub fn scale_to(&mut self, x: f64) {
        if self.is_percentage() || self.is_indexed_to_100() {
            return;
        }
        let Some(scale_start_point) = self.scale_start_point else {
            return;
        };

        let before = (self.options.auto_scale, self.price_range);
        self.options.auto_scale = false;

        let x = (self.height - (x - self.pane_offset)).max(0.0);

        let mut scale_coeff =
            (scale_start_point + (self.height - 1.0) * 0.2) / (x + (self.height - 1.0) * 0.2);
        // Set together with `scale_start_point` in `start_scale`; bail if the pair desynced.
        let Some(mut new_price_range) = self.price_range_snapshot else {
            return;
        };

        scale_coeff = scale_coeff.max(0.1);
        new_price_range.scale_around_center(scale_coeff);
        self.price_range = Some(new_price_range);
        if before != (self.options.auto_scale, self.price_range) {
            self.changed();
        }
    }

    pub fn end_scale(&mut self) {
        if self.is_percentage() || self.is_indexed_to_100() {
            return;
        }
        self.scale_start_point = None;
        self.price_range_snapshot = None;
    }

    // --- wheel zoom (industry-standard; the reference has no price-axis wheel) ---

    /// Zoom the range by `factor` anchored at the price under chart-content coordinate `y`
    /// (that price stays fixed on screen). Mirrors the drag-to-scale guards: percentage and
    /// indexed-to-100 modes no-op, and an empty scale has nothing to zoom. Disables autoscale,
    /// like any manual range edit.
    pub fn zoom(&mut self, y: f64, factor: f64) {
        if self.is_percentage() || self.is_indexed_to_100() {
            return;
        }
        if self.is_empty() {
            return;
        }
        let Some(range) = self.price_range else {
            return;
        };
        // base_value is only read by the percentage/indexed modes, which bail above.
        let anchor = self.coordinate_to_price(y, 0.0);
        let before = (self.options.auto_scale, self.price_range);
        self.options.auto_scale = false;
        let mut new_range = range;
        new_range.scale_around_point(anchor, factor);
        self.price_range = Some(new_range);
        if before != (self.options.auto_scale, self.price_range) {
            self.changed();
        }
    }

    // --- axis-drag scroll ---

    pub fn start_scroll(&mut self, x: f64) {
        if self.is_auto_scale() {
            return;
        }
        if self.scroll_start_point.is_some() || self.price_range_snapshot.is_some() {
            return;
        }
        if self.is_empty() {
            return;
        }
        self.scroll_start_point = Some(x - self.pane_offset);
        self.price_range_snapshot = self.price_range;
    }

    pub fn scroll_to(&mut self, x: f64) {
        if self.is_auto_scale() {
            return;
        }
        let Some(scroll_start_point) = self.scroll_start_point else {
            return;
        };

        // Both are set alongside `scroll_start_point` in `start_scroll`; bail if desynced.
        let (Some(price_range), Some(mut new_price_range)) =
            (self.price_range, self.price_range_snapshot)
        else {
            return;
        };
        let price_units_per_pixel = price_range.length() / (self.internal_height() - 1.0);
        let mut pixel_delta = (x - self.pane_offset) - scroll_start_point;
        if self.is_inverted() {
            pixel_delta = -pixel_delta;
        }

        let price_delta = pixel_delta * price_units_per_pixel;
        new_price_range.shift(price_delta);
        if self.price_range != Some(new_price_range) {
            self.price_range = Some(new_price_range);
            self.changed();
        }
    }

    pub fn end_scroll(&mut self) {
        if self.is_auto_scale() {
            return;
        }
        if self.scroll_start_point.is_none() {
            return;
        }
        self.scroll_start_point = None;
        self.price_range_snapshot = None;
    }

    // --- autoscale range application (tail of `_recalculatePriceRangeImpl`) ---

    /// Applies a merged source range (already in logical space for the current mode).
    /// `min_move` = 1/base of the formatter source (e.g. 0.01).
    pub fn apply_autoscale_range(&mut self, merged: Option<PriceRange>, min_move: f64) {
        self.apply_autoscale_ranges(merged, None, None, min_move);
    }

    /// Full autoscale application. `exact` is the merged source range over the strictly visible
    /// bars; `extended` is the same union one bar beyond each visible edge (only consulted in
    /// stable mode); `center` is `autoscale_center` already converted to this scale's logical
    /// domain. All ranges are in the current mode's logical space.
    pub fn apply_autoscale_ranges(
        &mut self,
        exact: Option<PriceRange>,
        extended: Option<PriceRange>,
        center: Option<f64>,
        min_move: f64,
    ) {
        let before = (self.min_move, self.price_range, self.log_formula);
        if min_move.is_finite() && min_move > 0.0 {
            self.min_move = min_move;
        }
        let Some(exact) = exact else {
            self.stable_range_valid = false;
            // reset empty to default
            if self.price_range.is_none() {
                self.price_range = Some(PriceRange::new(-0.5, 0.5));
                self.log_formula = log_formula::log_formula_for_price_range(None);
            }
            if before != (self.min_move, self.price_range, self.log_formula) {
                self.changed();
            }
            return;
        };
        // A center so far from the data that the mirrored range overflows keeps the plain range.
        let symmetric = |range: PriceRange| match center.filter(|value| value.is_finite()) {
            Some(center) => {
                let half = (range.max_value() - center)
                    .abs()
                    .max((range.min_value() - center).abs());
                let (low, high) = (center - half, center + half);
                if low.is_finite() && high.is_finite() {
                    PriceRange::new(low, high)
                } else {
                    range
                }
            }
            None => range,
        };
        let exact = symmetric(exact);
        let mut price_range = match (self.options.stable_auto_scale, self.price_range) {
            (true, Some(current)) if self.stable_range_valid => {
                // Grow at once so no visible bar is ever clipped.
                let grown = PriceRange::new(
                    current.min_value().min(exact.min_value()),
                    current.max_value().max(exact.max_value()),
                );
                // Shrink only toward data that is stable one bar past both visible edges,
                // clamped into the grown range so off-screen extremes never widen it.
                let settled = symmetric(extended.map_or(exact, |range| range.merge(Some(&exact))));
                let target = PriceRange::new(
                    settled.min_value().max(grown.min_value()),
                    settled.max_value().min(grown.max_value()),
                );
                if grown.length() - target.length()
                    > STABLE_AUTOSCALE_SHRINK_FRACTION * grown.length()
                {
                    target
                } else {
                    grown
                }
            }
            _ => exact,
        };
        self.stable_range_valid = self.options.stable_auto_scale;

        if price_range.min_value() == price_range.max_value() {
            // degenerate range: extend by 5 min-move values on each side (in raw space)
            let extend_value = 5.0 * min_move;
            if self.is_log() {
                price_range =
                    log_formula::convert_price_range_from_log(&price_range, &self.log_formula);
            }
            price_range = PriceRange::new(
                price_range.min_value() - extend_value,
                price_range.max_value() + extend_value,
            );
            if self.is_log() {
                price_range =
                    log_formula::convert_price_range_to_log(&price_range, &self.log_formula);
            }
        }

        if self.is_log() {
            let raw_range =
                log_formula::convert_price_range_from_log(&price_range, &self.log_formula);
            let new_formula = log_formula::log_formula_for_price_range(Some(&raw_range));
            if !log_formula::log_formulas_are_same(&new_formula, &self.log_formula) {
                let raw_snapshot = self
                    .price_range_snapshot
                    .map(|s| log_formula::convert_price_range_from_log(&s, &self.log_formula));
                self.log_formula = new_formula;
                price_range = log_formula::convert_price_range_to_log(&raw_range, &new_formula);
                if let Some(raw) = raw_snapshot {
                    self.price_range_snapshot =
                        Some(log_formula::convert_price_range_to_log(&raw, &new_formula));
                }
            }
        }

        self.price_range = Some(price_range);
        if before != (self.min_move, self.price_range, self.log_formula) {
            self.changed();
        }
    }

    // --- tick marks (port of PriceTickMarkBuilder) ---

    fn tick_mark_height(&self) -> f64 {
        (self.options.font_size * self.options.tick_mark_density).ceil()
    }

    /// Generates tick marks for the current range. `min_move` is the price grid every tick must
    /// lie on (the formatter source's minimum move, `0.01` for percentage/indexed scales).
    /// `entire_text_only_margin` should be `font_size / 2` when the entireTextOnly option is on,
    /// else 0. With `ensure_edge_tick_marks_visible` on an autoscaled scale, rounded boundary
    /// marks are added at the top and bottom edges (reference `_applyEdgeMarks`).
    pub fn build_tick_marks(&self, min_move: f64, entire_text_only_margin: f64) -> Vec<PriceMark> {
        self.build_tick_marks_on_grid(|_, _| min_move, entire_text_only_margin)
    }

    /// [`Self::build_tick_marks`] for a price grid that depends on the price region (a
    /// tick-size ladder): `grid(low, high)` returns the step every tick in the builder-domain
    /// interval `[low, high]` must be a multiple of (raw prices on a log scale, logical values
    /// otherwise). The grid of a sub-interval must divide the grid of any interval containing
    /// it. A log scale re-derives its span per mark over `[low, mark]`, so each lower price
    /// region keeps its own finer grid instead of the coarsest grid of the whole view.
    pub fn build_tick_marks_on_grid(
        &self,
        grid: impl Fn(f64, f64) -> f64,
        entire_text_only_margin: f64,
    ) -> Vec<PriceMark> {
        let mut marks = Vec::new();

        if self.is_empty() {
            return marks;
        }

        let scale_height = self.height;
        let bottom = self.coordinate_to_logical_raw(scale_height - 1.0);
        let top = self.coordinate_to_logical_raw(0.0);

        let min_coord = entire_text_only_margin;
        let max_coord = scale_height - 1.0 - entire_text_only_margin;

        let high = bottom.max(top);
        let low = bottom.min(top);
        if high == low {
            return marks;
        }

        let mut span = composite_tick_span(
            high,
            low,
            grid(low, high),
            scale_height,
            self.tick_mark_height(),
        );
        let first_span = span;
        let mut modulo = high % span;
        if modulo < 0.0 {
            modulo += span;
        }

        let sign = if high >= low { 1.0 } else { -1.0 };
        let mut prev_coord: Option<f64> = None;

        let mut logical = high - modulo;
        while logical > low {
            let coord = self.logical_to_coordinate_raw(logical);

            // skip marks that don't fit (required for log scale)
            let fits =
                prev_coord.is_none_or(|prev| (coord - prev).abs() >= self.tick_mark_height());
            let visible = coord >= min_coord && coord <= max_coord;

            if fits && visible {
                marks.push(PriceMark {
                    coord: coord + self.pane_offset,
                    logical,
                    edge: false,
                });
                prev_coord = Some(coord);
                if self.is_log() {
                    span = composite_tick_span(
                        logical * sign,
                        low,
                        grid(low, logical * sign),
                        scale_height,
                        self.tick_mark_height(),
                    );
                }
            }

            logical -= span;
        }

        if self.has_visible_edge_marks() && self.should_apply_edge_marks(first_span, low, high) {
            self.apply_edge_marks(&mut marks, &grid, first_span, min_coord, max_coord);
        }

        marks
    }

    /// reference `_shouldApplyEdgeMarks`: only when both scale margins are narrower than a span.
    fn should_apply_edge_marks(&self, span: f64, low: f64, high: f64) -> bool {
        let Some(mut range) = self.price_range else {
            return false;
        };
        if self.is_log() {
            range = log_formula::convert_price_range_from_log(&range, &self.log_formula);
        }
        range.min_value() - low < span && high - range.max_value() < span
    }

    /// reference `_applyEdgeMarks`: replace a regular mark closer than half a span to an edge
    /// with a rounded boundary mark at that edge.
    fn apply_edge_marks(
        &self,
        marks: &mut Vec<PriceMark>,
        grid: &impl Fn(f64, f64) -> f64,
        span: f64,
        min_coord: f64,
        max_coord: f64,
    ) {
        let padding = self.edge_marks_padding();
        let top = self.boundary_price_mark(grid, min_coord, padding, padding * 2.0);
        let bottom = self.boundary_price_mark(grid, max_coord, -padding * 2.0, -padding);
        let span_px = self.logical_to_coordinate_raw(0.0) - self.logical_to_coordinate_raw(span);
        if marks
            .first()
            .is_some_and(|mark| mark.coord - top.coord < span_px / 2.0)
        {
            marks.remove(0);
        }
        if marks
            .last()
            .is_some_and(|mark| bottom.coord - mark.coord < span_px / 2.0)
        {
            marks.pop();
        }
        marks.insert(0, top);
        marks.push(bottom);
    }

    /// reference `_computeBoundaryPriceMark`: a value near `coord` rounded to a fine grid (at
    /// least 0.1, aligned to the local price grid so the boundary label is a tradable price).
    fn boundary_price_mark(
        &self,
        grid: &impl Fn(f64, f64) -> f64,
        coord: f64,
        min_padding: f64,
        max_padding: f64,
    ) -> PriceMark {
        let average = (min_padding + max_padding) / 2.0;
        let first = self.coordinate_to_logical_raw(coord + min_padding);
        let second = self.coordinate_to_logical_raw(coord + max_padding);
        let min_move = grid(first.min(second), first.max(second));
        let span = composite_tick_span(
            first.max(second),
            first.min(second),
            min_move,
            self.height,
            self.tick_mark_height(),
        );
        let value_span = align_span_to_min_move(span.max(0.1), min_move);
        let value = self.coordinate_to_logical_raw(coord + average);
        let rounded = value - value % value_span;
        PriceMark {
            coord: self.logical_to_coordinate_raw(rounded) + self.pane_offset,
            logical: rounded,
            edge: true,
        }
    }

    /// coordinate -> logical *without* undoing the log transform (the tick mark builder works in
    /// the transformed space; matches the closures reference passes to `PriceTickMarkBuilder`).
    fn coordinate_to_logical_raw(&self, coordinate: f64) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        let inv_coordinate = self.inverted_coordinate(coordinate);
        let Some(range) = self.price_range.as_ref() else {
            return 0.0;
        };
        // the reference's builder converters go through _coordinateToLogical which applies fromLog;
        // rebuildTickMarks then walks in *price* space for log scales.
        let logical = range.min_value()
            + range.length()
                * ((inv_coordinate - self.bottom_margin_px()) / (self.internal_height() - 1.0));
        if self.is_log() {
            log_formula::from_log(logical, &self.log_formula)
        } else {
            logical
        }
    }

    fn logical_to_coordinate_raw(&self, logical: f64) -> f64 {
        self.logical_to_coordinate_local(logical)
    }

    /// Convenience: format-facing conversion used by tests and axis code.
    pub fn logical_to_price_pub(&self, logical: f64, base_value: f64) -> f64 {
        self.logical_to_price(logical, base_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale_with_range(height: f64, min: f64, max: f64) -> PriceScaleCore {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions::default());
        s.set_height(height);
        s.set_price_range(Some(PriceRange::new(min, max)));
        s
    }

    #[test]
    fn price_to_coordinate_with_margins() {
        // height 100, margins 20/10 -> internalHeight 70
        let s = scale_with_range(100.0, 0.0, 10.0);
        // price 10 (top of range): inv = 10 + 69*1 = 79 -> y = 100-1-79 = 20 (= top margin)
        assert_eq!(s.price_to_coordinate(10.0, 10.0), 20.0);
        // price 0 (bottom): inv = 10 -> y = 89
        assert_eq!(s.price_to_coordinate(0.0, 10.0), 89.0);
    }

    #[test]
    fn coordinate_roundtrip() {
        let s = scale_with_range(300.0, 12.5, 87.5);
        for &p in &[12.5, 20.0, 55.5, 87.5] {
            let y = s.price_to_coordinate(p, p);
            let back = s.coordinate_to_price(y, p);
            assert!((back - p).abs() < 1e-9, "p={p} y={y} back={back}");
        }
    }

    #[test]
    fn wheel_zoom_keeps_the_cursor_price_fixed() {
        let mut s = scale_with_range(300.0, 12.5, 87.5);
        let anchor_price = 55.5;
        let anchor_y = s.price_to_coordinate(anchor_price, 0.0);
        // Zoom in (factor < 1 shrinks the range) and out again: the anchored price must stay
        // at the same coordinate, and autoscale drops like any manual range edit.
        s.zoom(anchor_y, 0.9);
        assert!(!s.options.auto_scale);
        let range_in = s.price_range().unwrap();
        assert!((range_in.length() - 75.0 * 0.9).abs() < 1e-9);
        assert!((s.coordinate_to_price(anchor_y, 0.0) - anchor_price).abs() < 1e-9);
        s.zoom(anchor_y, 1.0 / 0.9);
        let range_back = s.price_range().unwrap();
        assert!((range_back.length() - 75.0).abs() < 1e-8);
        assert!((range_back.min_value() - 12.5).abs() < 1e-8);
        assert!((s.coordinate_to_price(anchor_y, 0.0) - anchor_price).abs() < 1e-9);
    }

    #[test]
    fn wheel_zoom_no_ops_like_drag_to_scale() {
        // Percentage mode: no zoom (mirrors the drag-to-scale guard).
        let mut s = scale_with_range(300.0, 12.5, 87.5);
        s.options.mode = PriceScaleMode::Percentage;
        s.zoom(100.0, 0.9);
        assert!((s.price_range().unwrap().length() - 75.0).abs() < 1e-9);
        assert!(s.options.auto_scale);
    }

    #[test]
    fn inverted_scale_flips() {
        let mut s = scale_with_range(100.0, 0.0, 10.0);
        s.options.invert_scale = true;
        // inverted: margins swap and inv coordinate is used directly (y grows downward)
        // bottom_margin_px = scale_margins.top * h = 20; internal = 70
        // price 10 (max): inv = 20 + 69 = 89 -> highest price near the bottom
        assert_eq!(s.price_to_coordinate(10.0, 10.0), 89.0);
        assert_eq!(s.price_to_coordinate(0.0, 10.0), 20.0);
    }

    #[test]
    fn percentage_mode() {
        let mut s = scale_with_range(100.0, 0.0, 10.0); // range is in percent space
        s.options.mode = PriceScaleMode::Percentage;
        let base = 100.0;
        // price 110 -> +10% -> top of range
        assert_eq!(s.price_to_coordinate(110.0, base), 20.0);
        let p = s.coordinate_to_price(20.0, base);
        assert!((p - 110.0).abs() < 1e-9);
    }

    #[test]
    fn log_mode_roundtrip() {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions {
            mode: PriceScaleMode::Logarithmic,
            ..Default::default()
        });
        s.set_height(400.0);
        // store range in log space, as the model does
        let raw = PriceRange::new(1.0, 1000.0);
        let log_range = log_formula::convert_price_range_to_log(&raw, s.log_formula());
        s.set_price_range(Some(log_range));

        for &p in &[1.0, 10.0, 100.0, 999.0] {
            let y = s.price_to_coordinate(p, p);
            let back = s.coordinate_to_price(y, p);
            assert!((back - p).abs() < 1e-6, "p={p} back={back}");
        }
    }

    #[test]
    fn axis_drag_scale_matches_formula() {
        let mut s = scale_with_range(200.0, 0.0, 100.0);
        s.set_auto_scale(false);
        s.start_scale(150.0); // start point (inverted): 50
        s.scale_to(100.0); // x' = 100
                           // coeff = (50 + 199*0.2) / (100 + 199*0.2) = 89.8 / 139.8
        let coeff: f64 = 89.8 / 139.8;
        let r = s.price_range().unwrap();
        let expected_half = 50.0 * coeff;
        assert!((r.min_value() - (50.0 - expected_half)).abs() < 1e-9);
        assert!((r.max_value() - (50.0 + expected_half)).abs() < 1e-9);
        s.end_scale();
    }

    #[test]
    fn scroll_shifts_range_by_pixels() {
        let mut s = scale_with_range(100.0, 0.0, 69.0); // internalHeight-1 = 69 -> 1 price/px
        s.set_auto_scale(false);
        s.start_scroll(50.0);
        s.scroll_to(60.0); // +10 px down -> +10 price
        let r = s.price_range().unwrap();
        assert!((r.min_value() - 10.0).abs() < 1e-9);
        assert!((r.max_value() - 79.0).abs() < 1e-9);
    }

    #[test]
    fn degenerate_autoscale_range_expanded() {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions::default());
        s.set_height(100.0);
        s.apply_autoscale_range(Some(PriceRange::new(50.0, 50.0)), 0.01);
        let r = s.price_range().unwrap();
        assert!((r.min_value() - 49.95).abs() < 1e-12);
        assert!((r.max_value() - 50.05).abs() < 1e-12);
    }

    #[test]
    fn empty_autoscale_gets_default_range() {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions::default());
        s.set_height(100.0);
        s.apply_autoscale_range(None, 0.01);
        assert_eq!(s.price_range().unwrap(), &PriceRange::new(-0.5, 0.5));
    }

    #[test]
    fn tick_marks_are_spaced_and_round() {
        let mut s = scale_with_range(300.0, 0.0, 100.0);
        s.set_height(300.0);
        let marks = s.build_tick_marks(0.01, 0.0);
        assert!(!marks.is_empty());
        // marks must be at multiples of the span -> logical values divide evenly
        let span = (marks[0].logical - marks[1].logical).abs();
        for w in marks.windows(2) {
            assert!(((w[0].logical - w[1].logical).abs() - span).abs() < 1e-9);
        }
        // spacing respects tick mark height (30px for font 12, density 2.5)
        for w in marks.windows(2) {
            assert!((w[1].coord - w[0].coord).abs() >= 30.0 - 1e-9);
        }
        // coordinates within scale
        for m in &marks {
            assert!(m.coord >= 0.0 && m.coord <= 299.0);
        }
    }

    #[test]
    fn tick_labels_lie_on_every_min_move_grid_and_are_unique() {
        use crate::format::price_formatter::{precision_by_min_move, PriceFormatter};
        use crate::scale::price_tick_span_calculator::is_multiple_of;
        let min_moves = [0.02, 0.05, 0.005, 0.2, 1.0, 2.0, 5.0, 0.25, 0.03125];
        let centers = [0.8, 9.9, 15.0, 25.0, 97.5, 250.0, 1_234.0, 7_500.0];
        let widths = [0.07, 0.3, 1.43, 4.0, 17.0, 80.0, 333.0, 2_500.0];
        let heights = [120.0, 300.0, 400.0, 611.0, 900.0];
        let mut checked = 0;
        for min_move in min_moves {
            let precision = precision_by_min_move(min_move);
            let formatter = PriceFormatter::from_precision(precision, min_move);
            for mode in [PriceScaleMode::Normal, PriceScaleMode::Logarithmic] {
                for center in centers {
                    for width in widths {
                        if width < min_move * 4.0 || center - width <= 0.0 {
                            continue;
                        }
                        for height in heights {
                            let mut s = PriceScaleCore::new(PriceScaleCoreOptions {
                                mode,
                                ..Default::default()
                            });
                            s.set_height(height);
                            s.apply_autoscale_range(
                                Some(
                                    s.price_range_to_logical(
                                        &PriceRange::new(
                                            center - width / 2.0,
                                            center + width / 2.0,
                                        ),
                                        1.0,
                                    )
                                    .unwrap(),
                                ),
                                min_move,
                            );
                            let marks = s.build_tick_marks(min_move, 0.0);
                            let mut labels = Vec::with_capacity(marks.len());
                            for mark in &marks {
                                assert!(
                                    is_multiple_of(mark.logical, min_move),
                                    "{mode:?} min_move {min_move} center {center} width {width} \
                                     height {height}: tick {}",
                                    mark.logical
                                );
                                let label = formatter.format(mark.logical).replace(',', "");
                                let value: f64 = label.parse().unwrap();
                                assert!(
                                    is_multiple_of(value, min_move),
                                    "{mode:?} min_move {min_move}: label {label}"
                                );
                                labels.push(label);
                            }
                            let unique: std::collections::BTreeSet<_> = labels.iter().collect();
                            assert_eq!(
                                unique.len(),
                                labels.len(),
                                "{mode:?} min_move {min_move} center {center} width {width} \
                                 height {height}: duplicate labels {labels:?}"
                            );
                            checked += marks.len();
                        }
                    }
                }
            }
        }
        assert!(checked > 5_000, "sweep produced too few ticks: {checked}");
    }

    #[test]
    fn log_ticks_keep_each_price_regions_own_grid() {
        use crate::scale::price_tick_span_calculator::is_multiple_of;
        // A two-band ladder: 0.01 below 10, 1 from 10 up. An interval's grid is its coarsest
        // band, which every sub-interval's grid divides.
        let band = |price: f64| -> f64 {
            if price.abs() < 10.0 {
                0.01
            } else {
                1.0
            }
        };
        let grid = |low: f64, high: f64| band(low).max(band(high));
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions {
            mode: PriceScaleMode::Logarithmic,
            ..Default::default()
        });
        s.set_height(600.0);
        s.apply_autoscale_range(
            Some(
                s.price_range_to_logical(&PriceRange::new(0.3, 3_000.0), 1.0)
                    .unwrap(),
            ),
            0.01,
        );
        let marks = s.build_tick_marks_on_grid(grid, 0.0);
        for mark in &marks {
            assert!(
                is_multiple_of(mark.logical, band(mark.logical)),
                "{mark:?} is off its band grid"
            );
        }
        let low_region = marks.iter().filter(|mark| mark.logical < 10.0).count();
        assert!(
            low_region >= 3,
            "the sub-10 decades kept too few ticks: {marks:?}"
        );
        // The view-wide grid alone (1.0) cannot label anything below 1.
        let coarse = s.build_tick_marks(grid(0.3, 3_000.0), 0.0);
        assert!(coarse.iter().all(|mark| mark.logical >= 1.0));
        assert!(marks.iter().any(|mark| mark.logical < 1.0));
        // A constant grid is exactly `build_tick_marks`.
        assert_eq!(
            s.build_tick_marks_on_grid(|_, _| 0.01, 0.0),
            s.build_tick_marks(0.01, 0.0)
        );
    }

    #[test]
    fn edge_tick_marks_follow_the_reference_boundary_rule() {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions {
            scale_margins: PriceScaleMargins {
                top: 0.0,
                bottom: 0.0,
            },
            ..Default::default()
        });
        s.set_height(300.0);
        s.apply_autoscale_range(Some(PriceRange::new(101.37, 118.91)), 0.01);
        let plain = s.build_tick_marks(0.01, 0.0);
        assert!(plain.iter().all(|mark| !mark.edge));
        s.set_ensure_edge_tick_marks_visible(true);
        let marks = s.build_tick_marks(0.01, 0.0);
        let first = marks.first().unwrap();
        let last = marks.last().unwrap();
        assert!(first.edge && last.edge);
        assert!(marks[1..marks.len() - 1].iter().all(|mark| !mark.edge));
        // Boundary marks sit within the half-font edge padding band and on the 0.1 grid.
        assert!(
            first.coord >= 6.0 - 1.0 && first.coord <= 12.0 + 1.0,
            "{first:?}"
        );
        assert!(last.coord <= 299.0 - 6.0 + 1.0 && last.coord >= 299.0 - 12.0 - 1.0);
        for mark in [first, last] {
            assert!(
                crate::scale::price_tick_span_calculator::is_multiple_of(mark.logical, 0.1),
                "{mark:?}"
            );
        }
        // The padding is applied as an internal margin while autoscaled.
        assert!((s.price_to_coordinate(118.91, 0.0) - 6.0).abs() < 1e-9);
        // Manual scales drop edge marks, as in the reference.
        s.set_auto_scale(false);
        assert!(s.build_tick_marks(0.01, 0.0).iter().all(|mark| !mark.edge));
    }

    #[test]
    fn symmetric_autoscale_centers_the_range_on_the_requested_price() {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions::default());
        s.set_height(300.0);
        s.apply_autoscale_ranges(Some(PriceRange::new(101.0, 104.0)), None, Some(100.0), 0.01);
        assert_eq!(s.price_range().unwrap(), &PriceRange::new(96.0, 104.0));
        s.apply_autoscale_ranges(Some(PriceRange::new(97.5, 100.5)), None, Some(100.0), 0.01);
        assert_eq!(s.price_range().unwrap(), &PriceRange::new(97.5, 102.5));
        // Percentage-domain center (0%) works identically.
        s.apply_autoscale_ranges(Some(PriceRange::new(-1.5, 3.0)), None, Some(0.0), 0.01);
        assert_eq!(s.price_range().unwrap(), &PriceRange::new(-3.0, 3.0));
        // A center whose mirrored range would overflow keeps the finite data range.
        s.apply_autoscale_ranges(Some(PriceRange::new(1.0, 2.0)), None, Some(f64::MAX), 0.01);
        assert_eq!(s.price_range().unwrap(), &PriceRange::new(1.0, 2.0));
    }

    #[test]
    fn stable_autoscale_grows_at_once_shrinks_with_hysteresis_and_resets() {
        let mut s = PriceScaleCore::new(PriceScaleCoreOptions {
            stable_auto_scale: true,
            ..Default::default()
        });
        s.set_height(300.0);
        let apply = |s: &mut PriceScaleCore, exact: (f64, f64), extended: (f64, f64)| {
            s.apply_autoscale_ranges(
                Some(PriceRange::new(exact.0, exact.1)),
                Some(PriceRange::new(extended.0, extended.1)),
                None,
                0.01,
            );
            *s.price_range().unwrap()
        };
        // First pass: exact range.
        assert_eq!(
            apply(&mut s, (100.0, 110.0), (99.0, 111.0)),
            PriceRange::new(100.0, 110.0)
        );
        // A new visible extreme grows the range at once.
        assert_eq!(
            apply(&mut s, (100.0, 115.0), (100.0, 115.0)),
            PriceRange::new(100.0, 115.0)
        );
        // The extreme bar leaves the strict view but is still within one bar: no shrink.
        assert_eq!(
            apply(&mut s, (100.0, 110.0), (100.0, 115.0)),
            PriceRange::new(100.0, 115.0)
        );
        // Small unused headroom (below the threshold) is kept.
        assert_eq!(
            apply(&mut s, (100.5, 113.0), (100.5, 113.0)),
            PriceRange::new(100.0, 115.0)
        );
        // More than 20% unused: shrink to the settled data.
        assert_eq!(
            apply(&mut s, (104.0, 110.0), (103.0, 111.0)),
            PriceRange::new(103.0, 111.0)
        );
        // Explicit autoscale reset restarts from the exact range.
        s.set_auto_scale(true);
        assert_eq!(
            apply(&mut s, (105.0, 106.0), (104.0, 107.0)),
            PriceRange::new(105.0, 106.0)
        );
        s.reset_autoscale_stabilization();
        assert_eq!(
            apply(&mut s, (105.5, 106.0), (104.0, 107.0)),
            PriceRange::new(105.5, 106.0)
        );
        // Default (exact) mode follows every range immediately.
        let mut exact = PriceScaleCore::new(PriceScaleCoreOptions::default());
        exact.set_height(300.0);
        assert_eq!(
            apply(&mut exact, (100.0, 115.0), (100.0, 115.0)),
            PriceRange::new(100.0, 115.0)
        );
        assert_eq!(
            apply(&mut exact, (100.0, 110.0), (100.0, 115.0)),
            PriceRange::new(100.0, 110.0)
        );
    }

    #[test]
    fn scroll_requires_manual_scale_and_reset_cancels_the_session() {
        let mut s = scale_with_range(100.0, 0.0, 10.0);
        assert!(s.is_auto_scale());
        s.start_scroll(10.0);
        s.scroll_to(20.0);
        assert_eq!(s.price_range().unwrap(), &PriceRange::new(0.0, 10.0));

        s.set_auto_scale(false);
        s.start_scroll(10.0);
        s.scroll_to(20.0);
        assert_ne!(s.price_range().unwrap(), &PriceRange::new(0.0, 10.0));

        s.set_auto_scale(true);
        assert!(s.scroll_start_point.is_none());
        assert!(s.price_range_snapshot.is_none());

        let reset_range = *s.price_range().unwrap();
        s.set_auto_scale(false);
        s.scroll_to(30.0);
        assert_eq!(s.price_range().unwrap(), &reset_range);
    }

    #[test]
    fn clamp_helper_sanity() {
        use crate::helpers::mathex::clamp;
        assert_eq!(clamp(5.0, 0.0, 10.0), 5.0);
        assert_eq!(clamp(-5.0, 0.0, 10.0), 0.0);
        assert_eq!(clamp(50.0, 0.0, 10.0), 10.0);
    }
}
