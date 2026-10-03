//! Draw-list IR consumed by rendering backends.
//!
//! Two coordinate flavors, based on the media-space/device-space distinction shared by every
//! executor:
//! - integer **bitmap** rects (`Rect`, `RectFrame`, `HLine`, `VLine`) — crisp, no AA;
//! - float bitmap-space geometry (`Polyline`, `AreaFill`, `RoundRect`, `Circle`, `Text`) — AA'd.

use std::sync::Arc;

use crate::color::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineStyle {
    Solid,
    Dotted,
    Dashed,
}

impl LineStyle {
    /// Dash pattern in bitmap px for a given line width:
    /// `Dotted` is the SPARSE pattern and `Dashed` the LARGE one — the reference's normal
    /// dotted `[w, w]` and dashed `[2w, 2w]` patterns do not exist in this engine (dots too
    /// close / dashes too short), and neither do its `LargeDashed`/`SparseDotted` variants.
    pub fn dash_pattern(&self, line_width: f32) -> Vec<f32> {
        let w = line_width;
        match self {
            LineStyle::Solid => vec![],
            LineStyle::Dotted => vec![w, 4.0 * w],
            LineStyle::Dashed => vec![6.0 * w, 6.0 * w],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineType {
    Simple,
    WithSteps,
    Curved,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub top: Color,
    pub bottom: Color,
}

/// Horizontal alignment of a [`Prim::Text`] run around its anchor x.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl TextAlign {
    /// The Canvas `textAlign` keyword both browser backends set before drawing/measuring.
    pub fn canvas_keyword(&self) -> &'static str {
        match self {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        }
    }
}

/// CSS font shorthand for a text run: `"[{italic} ]{weight} {size}px {family}"` with `weight`
/// the numeric CSS font weight (100–900; 400 normal, 700 bold). `size` is in the IR's bitmap
/// px; `family` is the resolved family list (the layout `fontFamily` default is folded in by
/// the decoder). One string shared by the Canvas2D executor's `fillText` and the WebGPU host
/// rasterizer, so both draw the same glyphs.
pub fn text_font_spec(size: f32, family: &str, weight: u16, italic: bool) -> String {
    format!(
        "{}{} {}px {}",
        if italic { "italic " } else { "" },
        weight,
        size,
        family
    )
}

/// Immutable straight-alpha RGBA8 pixels shared by retained frames and every executor.
///
/// `key` is chart-local and immutable for the lifetime of the pixels. Backends use it to retain
/// decoded/uploaded image resources without hashing or copying the payload every frame.
#[derive(Clone, Debug)]
pub struct RasterImage {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// Snap each image destination edge to a device pixel, rounding exact half pixels toward zero.
/// GPUI applies this rule to polychrome sprites; all other executors use it before drawing.
pub fn snap_image_rect([x, y, width, height]: [f32; 4]) -> Option<[f32; 4]> {
    let right = x + width;
    let bottom = y + height;
    if width <= 0.0 || height <= 0.0 || ![x, y, right, bottom].iter().all(|value| value.is_finite())
    {
        return None;
    }
    let round = |value: f32| (value.abs() - 0.5).ceil().copysign(value);
    let (left, top, right, bottom) = (round(x), round(y), round(right), round(bottom));
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}

/// Convert straight RGBA8 to premultiplied RGBA8 before bilinear filtering.
pub fn premultiply_rgba8(pixels: &mut [u8]) {
    debug_assert_eq!(pixels.len() % 4, 0);
    for rgba in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(rgba[3]);
        for channel in &mut rgba[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

impl PartialEq for RasterImage {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.width == other.width && self.height == other.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    /// Integer bitmap-space filled rect (Canvas2D `fillRect` semantics).
    Rect { rect: IRect, color: Color },
    /// Hollow frame filled inside `rect` (Canvas2D `fillRectInnerBorder` semantics).
    RectFrame {
        rect: IRect,
        border: i32,
        color: Color,
    },
    /// Full-length 1px-class horizontal line at integer y (with half-pixel handling in backend).
    HLine {
        y: i32,
        x0: i32,
        x1: i32,
        width: i32,
        style: LineStyle,
        color: Color,
    },
    VLine {
        x: i32,
        y0: i32,
        y1: i32,
        width: i32,
        style: LineStyle,
        color: Color,
    },
    /// Anti-aliased polyline over `points[range]`, round joins / butt caps. A non-finite or
    /// non-positive width emits nothing on every executor.
    ///
    /// Every executor honors `style`: Canvas2D and native through their dash APIs, GPUI and WebGPU
    /// by splitting the expanded path with the shared [`crate::line::dash_split`], whose walk is
    /// bounded by `MAX_DASH_STEPS`. Producers still lower dashed and dotted strokes to solid runs
    /// through [`crate::line::push_styled_stroke`] (engine frame construction and the browser
    /// host's decoded plugin primitives both do), which clips the path to the pane before it is
    /// split, so the dash count follows the visible length and every executor receives identical
    /// dashes.
    Polyline {
        first_point: u32,
        point_count: u32,
        width: f32,
        style: LineStyle,
        line_type: LineType,
        color: Color,
    },
    /// A batch of independent two-point strokes over
    /// `points[first_point .. first_point + 2 * segment_count]`: pair `i` is
    /// `points[first_point + 2 * i]` to `points[first_point + 2 * i + 1]`. Each pair strokes exactly
    /// like a solid simple two-point [`Prim::Polyline`] of this `width` and `color` (anti-aliased,
    /// butt caps, no joins); a dashed line is already expanded into one pair per dash. The engine
    /// emits it for period-reset study lines on bars of a day or longer, where every bar is its own
    /// line run and would otherwise cost one `Polyline` (and one executor state change) per bar.
    ///
    /// The engine's pairs are ascending in x, each spans at most one bar, and neighbours may touch
    /// at their endpoints (overlapping by float rounding). An executor may stroke the batch as one
    /// path or one mesh; the only visible difference from separate strokes is at those shared
    /// boundary pixels, where a single-path stroke unions coverage instead of compositing twice.
    /// A range outside the point pool is dropped.
    Segments {
        first_point: u32,
        segment_count: u32,
        width: f32,
        color: Color,
    },
    /// Fill between polyline and a horizontal base with a vertical gradient.
    /// `line_type` matches the companion `Polyline` so stepped/curved areas trace the same edge.
    AreaFill {
        first_point: u32,
        point_count: u32,
        base_y: f32,
        line_type: LineType,
        gradient: Gradient,
    },
    /// Solid fill between two polylines over the same x sequence (Bollinger-style band
    /// fills). Both ranges index the shared point pool and hold `point_count` entries; the
    /// path closes upper-forward + lower-backward. `line_type` expands both boundaries at one
    /// coupled sample sequence so the fill stays aligned with its companion strokes.
    BandFill {
        upper_first: u32,
        lower_first: u32,
        point_count: u32,
        line_type: LineType,
        fill: Color,
    },
    RoundRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        /// left-top, right-top, right-bottom, left-bottom
        radii: [f32; 4],
        fill: Color,
        border_width: f32,
        border_color: Color,
    },
    /// Filled disc with an optional centered stroke. A non-finite or non-positive radius emits
    /// nothing, including its stroke, on every executor.
    Circle {
        cx: f32,
        cy: f32,
        radius: f32,
        fill: Color,
        stroke_width: f32,
        stroke: Color,
    },
    /// Filled triangle in bitmap space (markers/arrows and other small annotations).
    Triangle {
        a: [f32; 2],
        b: [f32; 2],
        c: [f32; 2],
        color: Color,
    },
    /// Vertical gradient over `rect` (bitmap px) — the reference `layout.background`
    /// `VerticalGradient` painted per pane (pane-widget.ts `_drawBackground` spans the pane's
    /// own bitmap, so a stacked pane each gets the full top→bottom ramp).
    Background { rect: [f32; 4], gradient: Gradient },
    /// A run of text anchored at (x, y) bitmap px: x is the aligned edge (`align`), y the
    /// vertical center (Canvas `textBaseline: "middle"`, the axis-label convention). `size` is
    /// bitmap px and `family` fully resolved (layout defaults folded in by the decoder), so
    /// [`text_font_spec`] yields the exact font string both browser backends rasterize with.
    Text {
        x: f32,
        y: f32,
        text: String,
        color: Color,
        size: f32,
        family: String,
        align: TextAlign,
        /// Numeric CSS font weight (100–900; 400 normal, 700 bold).
        weight: u16,
        italic: bool,
    },
    /// A text run rotated clockwise around its aligned `(x, y)` anchor. Used by geometry whose
    /// label follows a segment; keeping the angle in the ordered frame makes every executor obey
    /// the same placement instead of reconstructing drawing semantics in a backend.
    RotatedText {
        x: f32,
        y: f32,
        text: String,
        color: Color,
        size: f32,
        family: String,
        align: TextAlign,
        weight: u16,
        italic: bool,
        angle: f32,
    },
    /// Straight-alpha RGBA8 image scaled into `rect` in bitmap pixels. Executors snap each
    /// destination edge to a device pixel with [`snap_image_rect`] and sample with bilinear
    /// filtering. Opacity multiplies source alpha before source-over blending. Resource decoding
    /// belongs to the host boundary; placement and rendering remain in the shared frame.
    Image {
        image: RasterImage,
        rect: [f32; 4],
        opacity: f32,
    },
}

/// The `2 * segment_count` pool points of a [`Prim::Segments`] batch, or `None` when the range
/// leaves the pool, which every executor treats as a dropped prim (as it does a short `Polyline`
/// range). The bound is checked `usize` math: a `u32` sum would wrap on 64-bit hosts' pool
/// indices, and `usize` itself is 32 bits on wasm32.
pub fn segment_points(
    points: &[[f32; 2]],
    first_point: u32,
    segment_count: u32,
) -> Option<&[[f32; 2]]> {
    let start = usize::try_from(first_point).ok()?;
    let len = usize::try_from(segment_count).ok()?.checked_mul(2)?;
    points.get(start..start.checked_add(len)?)
}

/// Geometry admission rule shared by Canvas2D, WebGPU, GPUI, and native CPU execution.
#[inline]
pub fn positive_finite_extent(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

/// One pane's frame output: `main` is redrawn on Light/Full invalidation; `top`
/// (crosshair + top primitives) is redrawn on every Cursor invalidation.
#[derive(Clone, Debug, Default)]
pub struct PaneLayers {
    pub main: Vec<Prim>,
    pub top: Vec<Prim>,
    /// Shared point pool referenced by Polyline/AreaFill ranges.
    pub points: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub panes: Vec<PaneLayers>,
    pub time_axis: PaneLayers,
    pub left_price_axes: Vec<PaneLayers>,
    pub right_price_axes: Vec<PaneLayers>,
}

#[cfg(test)]
mod tests {
    use super::{premultiply_rgba8, segment_points, snap_image_rect};

    #[test]
    fn segment_window_is_bounded_by_the_pool() {
        let pool = [
            [0.0f32, 0.0],
            [1.0, 0.0],
            [2.0, 1.0],
            [3.0, 1.0],
            [9.0, 9.0],
        ];
        assert_eq!(segment_points(&pool, 0, 2), Some(&pool[0..4]));
        assert_eq!(segment_points(&pool, 2, 1), Some(&pool[2..4]));
        assert_eq!(segment_points(&pool, 1, 2), Some(&pool[1..5]));
        assert_eq!(segment_points(&pool, 4, 0), Some(&pool[4..4]));
        assert_eq!(segment_points(&pool, 4, 1), None, "one point short");
        assert_eq!(segment_points(&pool, 6, 0), None, "start past the pool");
        assert_eq!(segment_points(&pool, u32::MAX, 1), None, "no wrap past u32");
        assert_eq!(
            segment_points(&pool, 1, u32::MAX),
            None,
            "no wrap on the pair count"
        );
    }

    #[test]
    fn image_rect_rounds_each_edge_with_gpui_half_toward_zero_rule() {
        assert_eq!(
            snap_image_rect([10.25, 20.75, 9.5, 9.5]),
            Some([10.0, 21.0, 10.0, 9.0])
        );
        assert_eq!(
            snap_image_rect([-0.5, -1.5, 20.0, 20.0]),
            Some([-0.0, -1.0, 19.0, 19.0])
        );
        assert_eq!(snap_image_rect([0.0, 0.0, 0.4, 20.0]), None);
    }

    #[test]
    fn image_premultiplication_preserves_opaque_and_scales_translucent_channels() {
        let mut pixels = [255, 128, 0, 128, 20, 40, 60, 255];
        premultiply_rgba8(&mut pixels);
        assert_eq!(pixels, [128, 64, 0, 128, 20, 40, 60, 255]);
    }
}
