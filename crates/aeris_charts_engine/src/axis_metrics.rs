//! Shared engine-owned axis metrics: one definition for strip sizes, label boxes, and text
//! sizes across layout, label construction, host text measurement, and hit geometry.
//!
//! Renderers stay executors — they consume the prepared [`AxisLabel`](crate::AxisLabel)
//! geometry (including [`AxisLabel::font_scale`](crate::AxisLabel)) without recomputing it.
//! Hosts measure text at the resolved sizes below (never at `layout.fontSize` with the painted
//! result shrunk), passing weight through so bold labels measure bold.
//!
//! Resolved sizes: axis-attached text runs at 11/12 of `layout.fontSize` (11 CSS px at the
//! 12 px default) with the configured family and proportional scaling for larger user fonts;
//! countdown text runs at 10/11 of the axis size (10 CSS px by default). Opacity is never
//! reduced to imitate a thinner font.

/// Axis-attached text scale relative to `layout.fontSize`.
pub(crate) const AXIS_FONT_SCALE: f64 = 11.0 / 12.0;
/// Countdown text scale relative to `layout.fontSize` (10/11 of the axis size).
pub(crate) const COUNTDOWN_FONT_SCALE: f64 = 10.0 / 12.0;

/// Fallback `layout.fontSize` when the configured value is non-finite or non-positive.
const DEFAULT_LAYOUT_FONT_SIZE: f64 = 12.0;

/// Resolved axis metrics for one `layout.fontSize`. All axis geometry derives from these —
/// price-strip chrome, tag boxes, cluster rows, the time strip, tick stubs, and the text sizes
/// hosts must measure at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AxisMetrics {
    /// Resolved axis text size: 11/12 of layout (11 CSS px by default).
    pub axis: f64,
    /// Resolved countdown text size: 10/12 of layout (10 CSS px by default).
    pub countdown: f64,
}

impl AxisMetrics {
    /// Resolve metrics from the configured `layout.fontSize`, falling back to the default
    /// for non-finite or non-positive input.
    pub fn new(layout_font_size: f64) -> Self {
        let layout = if layout_font_size.is_finite() && layout_font_size > 0.0 {
            layout_font_size
        } else {
            DEFAULT_LAYOUT_FONT_SIZE
        };
        Self {
            axis: layout * AXIS_FONT_SCALE,
            countdown: layout * COUNTDOWN_FONT_SCALE,
        }
    }

    /// Price-strip layout reservation at the pane edge. The visible rule itself uses the
    /// canonical design-system border width when the axis frame is lowered to device pixels.
    pub const PRICE_BORDER: f64 = 1.0;
    /// Tick allowance plus inner padding between the border and tag text.
    pub const PRICE_TEXT_GAP: f64 = 3.0 + 4.0;
    /// Outer padding between tag text and the strip edge.
    pub const PRICE_PAD_OUTER: f64 = 4.0;
    /// Price-strip chrome around the widest required text: border + gap + outer padding.
    pub const PRICE_CHROME: f64 = Self::PRICE_BORDER + Self::PRICE_TEXT_GAP + Self::PRICE_PAD_OUTER;

    /// Text inset from the strip border for tick labels and tag text: border + gap.
    pub const PRICE_TEXT_INSET: f64 = Self::PRICE_BORDER + Self::PRICE_TEXT_GAP;

    /// Price tag height: axis text plus 2 px padding above and below (15 CSS px by default).
    pub fn price_tag_height(&self) -> f64 {
        self.axis + 2.0 * 2.0
    }

    /// Crosshair price-tag height: the ordinary price tag plus 2 px of additional padding on
    /// both sides (19 CSS px by default), matching its distinct reference treatment without
    /// changing any other price-attached chip.
    pub fn crosshair_price_tag_height(&self) -> f64 {
        self.price_tag_height() + 4.0
    }

    /// Countdown row height: countdown text plus 2 px padding above and below (14 CSS px by
    /// default).
    pub fn countdown_row_height(&self) -> f64 {
        self.countdown + 2.0 * 2.0
    }

    /// Time-strip height: axis text plus the stable 1 px border slot, 3 px tick allowance, and 3 px
    /// vertical padding on each side, snapped to an even CSS-pixel height (22 CSS px by default).
    pub fn time_strip_height(&self) -> f64 {
        even_css_px(self.axis + 1.0 + 3.0 + 3.0 + 3.0)
    }

    /// Vertical center of time-strip text relative to the pane bottom edge: border, tick,
    /// and padding below the strip's vertical middle.
    pub fn time_text_dy(&self) -> f64 {
        1.0 + 3.0 + 3.0 + self.axis / 2.0
    }

    /// Time-tag horizontal padding per side; tags fit the resolved time-strip height.
    pub const TIME_TAG_PAD_X: f64 = 6.0;

    /// Shared corner radius for axis-attached price, time, drawing, alert, and live-value chips.
    pub const TAG_RADIUS: f64 = 1.0;

    /// Tick stub length painted at the strip edge.
    pub const TICK_LENGTH: f64 = 3.0;

    /// Text-width floor when a strip has no measurable labels.
    pub const DEFAULT_TEXT_WIDTH: f64 = 34.0;

    /// Full price-strip width from the widest required text: chrome plus text, ceiled, floored
    /// at the scale's `minimum_width`, snapped to an even CSS-pixel width.
    pub fn price_strip_width(text_width: f64, minimum_width: f64) -> f64 {
        let width = (Self::PRICE_CHROME + text_width).ceil().max(minimum_width);
        width + (width as i64 % 2) as f64
    }

    /// Price-tag box width for a measured label advance.
    pub fn price_tag_width(text_width: f64) -> f64 {
        Self::PRICE_CHROME + text_width
    }

    /// Time-tag box width for a measured label advance.
    pub fn time_tag_width(text_width: f64) -> f64 {
        text_width + Self::TIME_TAG_PAD_X * 2.0
    }
}

/// Snap up to an even CSS-pixel value.
fn even_css_px(value: f64) -> f64 {
    let ceiled = value.ceil();
    ceiled + (ceiled as i64 % 2) as f64
}

impl crate::ChartEngine {
    /// Resolved metrics for the current `layout.fontSize`.
    pub(crate) fn axis_metrics(&self) -> AxisMetrics {
        AxisMetrics::new(self.options.get().layout.font_size)
    }

    /// Resolved axis-attached text size in CSS px (11 at the 12 px default). Hosts measure
    /// axis labels (ticks, tags, clusters, crosshair, price-line, drawing, trading/alert tags)
    /// at this size with matching weight.
    pub fn axis_font_size(&self) -> f64 {
        self.axis_metrics().axis
    }

    /// Reference cap for a time-axis label, measured at the font size actually painted.
    pub fn axis_label_width_cap(&self) -> f64 {
        (self.axis_font_size() + 4.0) * 5.0 / 8.0
            * f64::from(self.tick_mark_max_character_length.max(1))
    }

    /// Resolved countdown text size in CSS px (10 at the default). Hosts measure countdown
    /// strings at this size; countdown advances are never derived by shrinking axis-size
    /// measurements.
    pub fn countdown_font_size(&self) -> f64 {
        self.axis_metrics().countdown
    }

    /// Sync every owned price scale's internal tick sizing from the resolved axis metrics.
    /// Layout font owns tick density; the core option is not host-configurable (no patch key,
    /// not serialized), so this write can never clobber host state. Scale revisions advance on
    /// a real change so coordinate-dependent layers rebuild; every caller already runs under
    /// invalidating input (options/pane/layout changes), making the extra fan-out a no-op.
    pub(crate) fn sync_axis_tick_fonts(&mut self) {
        let size = self.axis_metrics().axis;
        for pane in &mut self.panes {
            for scale in pane.scales_mut() {
                scale.set_tick_font_size(size);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sizes_match_the_compact_spec() {
        let metrics = AxisMetrics::new(12.0);
        assert_eq!(metrics.axis, 11.0);
        assert_eq!(metrics.countdown, 10.0);
        // Price strip chrome keeps its 1 px border slot + 3 tick + 4 + 4 padding. Visible border
        // thickness is a paint token and does not perturb the chart's established layout geometry.
        assert_eq!(AxisMetrics::PRICE_CHROME, 12.0);
        assert_eq!(AxisMetrics::PRICE_TEXT_INSET, 8.0);
        // Price tags: 11 + 2 + 2. Crosshair price tags add 2 px per side.
        // Countdown rows: 10 + 2 + 2.
        assert_eq!(metrics.price_tag_height(), 15.0);
        assert_eq!(metrics.crosshair_price_tag_height(), 19.0);
        assert_eq!(metrics.countdown_row_height(), 14.0);
        // Time strip: 11 + 1 + 3 + 3 + 3 = 21, even-snapped to 22.
        assert_eq!(metrics.time_strip_height(), 22.0);
        assert_eq!(metrics.time_text_dy(), 1.0 + 3.0 + 3.0 + 11.0 / 2.0);
        // Strip widths: chrome plus text, ceiled, minimum-floored, even-snapped.
        assert_eq!(AxisMetrics::price_strip_width(35.0, 0.0), 48.0);
        assert_eq!(AxisMetrics::price_strip_width(0.0, 64.0), 64.0);
        assert_eq!(AxisMetrics::price_tag_width(35.0), 47.0);
        assert_eq!(AxisMetrics::time_tag_width(40.0), 52.0);
        assert_eq!(AxisMetrics::TAG_RADIUS, 1.0);
        assert_eq!(AxisMetrics::TICK_LENGTH, 3.0);
    }

    #[test]
    fn larger_fonts_scale_proportionally_and_stay_even() {
        let metrics = AxisMetrics::new(20.0);
        assert_eq!(metrics.axis, 20.0 * 11.0 / 12.0);
        assert_eq!(metrics.countdown, 20.0 * 10.0 / 12.0);
        assert_eq!(metrics.price_tag_height(), 20.0 * 11.0 / 12.0 + 4.0);
        assert_eq!(
            metrics.crosshair_price_tag_height(),
            20.0 * 11.0 / 12.0 + 8.0
        );
        assert_eq!(metrics.countdown_row_height(), 20.0 * 10.0 / 12.0 + 4.0);
        let height = metrics.time_strip_height();
        assert_eq!(height % 2.0, 0.0, "time strip stays even-snapped");
        assert!(height >= metrics.axis + 10.0);
        assert_eq!(AxisMetrics::price_strip_width(10.0, 0.0) % 2.0, 0.0);
    }

    #[test]
    fn malformed_layout_fonts_fall_back_to_the_default() {
        for bad in [f64::NAN, f64::INFINITY, 0.0, -3.0] {
            let metrics = AxisMetrics::new(bad);
            assert_eq!(metrics.axis, 11.0);
            assert_eq!(metrics.countdown, 10.0);
            assert_eq!(metrics.time_strip_height(), 22.0);
        }
    }
}
