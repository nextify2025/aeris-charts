//! Compile-time drawing-tool catalog.
//!
//! Tool semantics belong here rather than in browser/native hosts.  The catalog is deliberately
//! static: Aeris needs one deterministic implementation shared by every backend, not a runtime
//! plugin registry.  New built-in tools should describe their placement/editing invariants here
//! and keep only genuinely tool-specific geometry/math in the drawing engine.
//!
//! Wire ids: `0..=84` is AerisTerminal upstream's contiguous catalog, in upstream order, and
//! upstream appends new tools from 85. The own-line tools (not in upstream) take the top of the
//! `u8` space, `240..=246`, so upstream's contiguous growth never collides with them. Ids only
//! cross the JS/wasm boundary in-process; persisted documents carry tool names, so an id is never
//! stored. Legacy fork tool names are input-only aliases (`LEGACY_DRAWING_KIND_NAMES`).
//!
//! Renderer ownership follows `spec().family`: `None` for every upstream kind except the three
//! measuring ranges (wire ids `13..=15`), which take upstream's `geometry.rs` body resolver, frame
//! arm, and hit code; `Some` for those three ranges and the own-line tools, whose constants live in
//! their `kinds/<family>.rs` module.

// ponytail: fork renderer extras retired by the upstream B8 sync and not yet re-applied on
// upstream's lowering (their options stay stored but inert). Documents the fork wrote, and the
// fork-era clipboard and sync items that prove where they came from, already carry each one's fork
// default (`kinds::legacy_fork_tool_options` and the legacy defaults), so they regain the fork look
// as each is re-applied. Lines: the one engine-formatted stats box every line tool's `labels`
// rendered as (InfoLine's five-stat box and stats_position included; each visible label now prints
// on its own line through upstream's label path, with the date range and duration still
// engine-formatted), TrendAngle's arc and reference line, the fork arrowhead geometry, a ray's
// `extend_left`, and the vertical extension of info_line, trend_angle and arrow_line by `extend_*`.
// Channels: the parallel channel's middle line, the crossing fill of a fork `flat_top_bottom` whose
// level crosses its base (it restores as the disjoint channel with that level as its second line),
// regression asymmetric and toggled deviations (stored per-side overrides; a fork document's band
// also folds into `regression_deviations` at the wider enabled side), OHLC source selection,
// Pearson's R, fit-line handles, time-only regression moves (upstream's anchors are free handles,
// so a regression moves on both axes), the regression's dashed centre line in `middle_color` (a
// fork document's trend already carries `middle_line`), its `extend_*`, and its zones as drag
// surfaces while selected. Fibonacci: per-level palette lines, dashed trend line, fan grid, full
// circles, vertical label alignment, phi spiral, level labels and selected bands as hit targets,
// the fib channel's `extend_*`, and 0.25 px ring and arc tessellation. Pitchforks and Gann: zone
// fills as selected hit targets, base-midpoint handle, Gann box time levels and angles, square
// stats box, fan scale ratio, fixed-square size ratio and corner handle, price-basis rescale.
// Patterns: harmonic ratio connectors and labels, point labels as hit targets, shaded XABCD
// triangles, head-and-shoulders neckline, triangle apex extension, 12-degree Elliott notation,
// show_wave, progressive previews. Annotations: projection sector, note pin and reveal-on-focus,
// the price note's boxed price, speech bubbles, the default texts of new fork-form annotations
// ("Note", "Callout", ...), signpost pole and its editor on placement (upstream's signpost is a
// two-anchor marker that opens none), arrow-mark text, multi-line family boxes for
// note/comment/callout/price_note/anchored_text, bars-pattern LOD aggregation, the forecast's
// source and target boxes (absolute change, Success/Failure on market colors, the box as a hit
// target; the target time stays, one line above upstream's outcome label). Shapes:
// rotated-rectangle width handles, ellipse bounds handles, on-curve anchors with tangent extension
// and chord fills, closed polylines, clip-aware flattening, end caps on arc, curve and double_curve
// from `stroke_start`/`stroke_end`, and the rotated rectangle's and triangle's outline as one
// seamless stroke. Re-applied on upstream's lowering instead: channel `extend_*`, the callout's tip
// and box handles, the highlighter's once-filled tube, and the regression trend's dashed anchor
// segment while it has no fit. Not restored, by owner decision (they would change upstream's anchor
// or option contracts): a ray turned into a segment and the extended line's `extend_*` toggles, the
// projection's independent sector radius (its third anchor), the price note's leader and label
// offset (its second anchor), the bars pattern's box fit, the symmetric rotated rectangle placed
// around its center axis, and a numeric fixed-square size. Also kept as upstream draws them
// (docs/api/compatibility.md): the fork's boxed pattern point labels above highs and below lows,
// the speed fan's time rays, ring, arc and wedge label placement, exact log-scale fib prices, the
// pitchfork's A-B swing and B-C handle guides and always-red median, the Gann box's four-side
// labels, the straighten modes, the fork's band and zone alphas, and the regression's sample
// deviation.

use super::kinds::DrawingFamily;
use super::DrawingKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingPlacement {
    /// Place a fixed number of anchors from ordinary click/tap activations.
    ClickAnchors { count: u8 },
    /// One click commits a tool-specific preset geometry around that semantic origin. The preset
    /// owns its generated defining points; the host still forwards an ordinary activation.
    SingleClickPreset { points: u8 },
    /// Place a fixed number of anchors immediately from pointer press.  This is currently the text
    /// tool so the platform editor can open without a trailing compatibility click.
    PressAnchors { count: u8 },
    /// Repeated clicks/taps append anchors until an explicit finish action.
    MultiClick { minimum: u8 },
    /// Pointer-down / move / pointer-up capture with engine-owned sample decimation.
    Freehand { minimum: u8 },
}

impl DrawingPlacement {
    pub(crate) const fn minimum_points(self) -> usize {
        match self {
            Self::ClickAnchors { count }
            | Self::PressAnchors { count }
            | Self::SingleClickPreset { points: count } => count as usize,
            Self::MultiClick { minimum } | Self::Freehand { minimum } => minimum as usize,
        }
    }

    pub(crate) const fn valid_point_count(self, count: usize) -> bool {
        match self {
            Self::ClickAnchors { count: exact }
            | Self::PressAnchors { count: exact }
            | Self::SingleClickPreset { points: exact } => count == exact as usize,
            Self::MultiClick { minimum } | Self::Freehand { minimum } => count >= minimum as usize,
        }
    }

    pub(crate) const fn is_sequence(self) -> bool {
        matches!(self, Self::MultiClick { .. })
    }

    pub(crate) const fn is_freehand(self) -> bool {
        matches!(self, Self::Freehand { .. })
    }

    pub(crate) const fn places_on_press(self) -> bool {
        matches!(self, Self::PressAnchors { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingHandleMode {
    None,
    Anchors,
    Endpoints,
    RectangleBounds,
    Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingMovementAxis {
    Both,
    HorizontalOnly,
    VerticalOnly,
}

impl DrawingMovementAxis {
    pub(crate) const fn constrain(self, dx: f64, dy: f64) -> (f64, f64) {
        match self {
            Self::Both => (dx, dy),
            Self::HorizontalOnly => (dx, 0.0),
            Self::VerticalOnly => (0.0, dy),
        }
    }

    /// `point` with the coordinates this axis moves taken from `snapped` (a magnet result).
    pub(crate) fn constrain_snap(
        self,
        point: super::DrawingPoint,
        snapped: super::DrawingPoint,
    ) -> super::DrawingPoint {
        match self {
            Self::Both => snapped,
            Self::HorizontalOnly => super::DrawingPoint {
                logical: snapped.logical,
                ..point
            },
            Self::VerticalOnly => super::DrawingPoint {
                price: snapped.price,
                ..point
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingStraightenMode {
    None,
    Segment45,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingLogicalExtent {
    Finite,
    Full,
    FromFirst,
    Ray,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingPriceExtent {
    Finite,
    Full,
}

#[derive(Clone, Copy)]
pub(crate) struct DrawingToolSpec {
    pub(crate) kind: DrawingKind,
    pub(crate) wire_id: u8,
    pub(crate) name: &'static str,
    pub(crate) placement: DrawingPlacement,
    pub(crate) handles: DrawingHandleMode,
    pub(crate) movement_axis: DrawingMovementAxis,
    pub(crate) straighten: DrawingStraightenMode,
    pub(crate) logical_extent: DrawingLogicalExtent,
    pub(crate) price_extent: DrawingPriceExtent,
    /// Conservative semantic-bounds expansion for curved/freehand interpolation.
    pub(crate) bounds_padding_ratio: f64,
    pub(crate) default_width: f64,
    /// Placement commits directly into a platform text-edit session.  The editor itself remains a
    /// host concern, but the decision that this tool requests one is canonical engine metadata.
    pub(crate) requests_text_editor: bool,
    /// B8 family hooks (`kinds/<family>.rs`). `None` keeps the core tool on the `geometry.rs`
    /// body resolver; `Some` routes body, decorations, and family labels through shared parts.
    pub(crate) family: Option<&'static DrawingFamily>,
    /// Reference the common text label resolves its 3×3 alignment against.
    pub(crate) text_layout: DrawingTextLayout,
    /// Paint the first anchor's price as a tag on the owning price axis (horizontal-line idiom).
    pub(crate) axis_price_label: bool,
    /// Anchors land on the crosshair's time slot and the instrument/scale price tick during
    /// creation, anchor drags, and body moves, so derived statistics read whole bars and ticks.
    pub(crate) grid_snap: bool,
    /// A coordinate every anchor shares (a horizontal segment's price, a vertical ray's bar).
    pub(crate) anchor_link: DrawingAnchorLink,
    /// The axis tag (`axis_price_label`) shows the drawing's `text` instead of its price when it
    /// has any, and the text is not painted on the chart (KLineChart's simple tag).
    pub(crate) axis_tag_text: bool,
}

/// A coordinate every anchor of a drawing shares. Placing, dragging, or supplying one anchor moves
/// that coordinate on the others, so the shape cannot leave its axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DrawingAnchorLink {
    #[default]
    None,
    /// All anchors share one price (a horizontal segment).
    SamePrice,
    /// All anchors share one logical index (a vertical ray or segment).
    SameLogical,
}

impl DrawingAnchorLink {
    /// Give every point the linked coordinate of `points[source]`.
    pub(crate) fn apply(self, points: &mut [super::DrawingPoint], source: usize) {
        let Some(&anchor) = points.get(source) else {
            return;
        };
        for point in points {
            match self {
                Self::None => {}
                Self::SamePrice => point.price = anchor.price,
                Self::SameLogical => point.logical = anchor.logical,
            }
        }
    }
}

/// How the common text label is placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingTextLayout {
    /// Against the resolved geometry's reference box (unrotated `Prim::Text`).
    Box,
    /// Along the first two anchors: slots follow the readable segment, the run rotates with it
    /// (`Prim::RotatedText`), follows the stroke color, and a middle label splits the stroke.
    Segment,
}

const TREND_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::TrendLine,
    wire_id: 0,
    name: "trend_line",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Segment,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const HORIZONTAL_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::HorizontalLine,
    wire_id: 1,
    name: "horizontal_line",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::VerticalOnly,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: true,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const HORIZONTAL_RAY: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::HorizontalRay,
    wire_id: 2,
    name: "horizontal_ray",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::FromFirst,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: true,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const VERTICAL_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::VerticalLine,
    wire_id: 3,
    name: "vertical_line",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::HorizontalOnly,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const RECTANGLE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Rectangle,
    wire_id: 4,
    name: "rectangle",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::RectangleBounds,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Square,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const TEXT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Text,
    wire_id: 5,
    name: "text",
    placement: DrawingPlacement::PressAnchors { count: 1 },
    handles: DrawingHandleMode::None,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: true,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const BRUSH: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Brush,
    wire_id: 6,
    name: "brush",
    placement: DrawingPlacement::Freehand { minimum: 2 },
    handles: DrawingHandleMode::Endpoints,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.25,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const PATH: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Path,
    wire_id: 7,
    name: "path",
    placement: DrawingPlacement::MultiClick { minimum: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const LONG_POSITION: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::LongPosition,
    wire_id: 8,
    name: "long_position",
    placement: DrawingPlacement::SingleClickPreset { points: 3 },
    handles: DrawingHandleMode::Position,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: true,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const SHORT_POSITION: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ShortPosition,
    wire_id: 9,
    name: "short_position",
    placement: DrawingPlacement::SingleClickPreset { points: 3 },
    handles: DrawingHandleMode::Position,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: true,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const FIXED_RANGE_VOLUME_PROFILE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FixedRangeVolumeProfile,
    wire_id: 10,
    name: "fixed_range_volume_profile",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::HorizontalOnly,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const ANCHORED_VOLUME_PROFILE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::AnchoredVolumeProfile,
    wire_id: 11,
    name: "anchored_volume_profile",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::HorizontalOnly,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::FromFirst,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const ANCHORED_VWAP: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::AnchoredVwap,
    wire_id: 12,
    name: "anchored_vwap",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::HorizontalOnly,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::FromFirst,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: None,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

const fn line_spec(kind: DrawingKind, wire_id: u8, name: &'static str) -> DrawingToolSpec {
    DrawingToolSpec {
        kind,
        wire_id,
        name,
        ..TREND_LINE
    }
}

const RAY: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Ray,
    price_extent: DrawingPriceExtent::Full,
    ..line_spec(DrawingKind::Ray, 16, "ray")
};
const EXTENDED_LINE: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..line_spec(DrawingKind::ExtendedLine, 17, "extended_line")
};
const INFO_LINE: DrawingToolSpec = line_spec(DrawingKind::InfoLine, 18, "info_line");
const TREND_ANGLE: DrawingToolSpec = line_spec(DrawingKind::TrendAngle, 19, "trend_angle");
const CROSS_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::CrossLine,
    wire_id: 20,
    name: "cross_line",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: true,
    ..TREND_LINE
};
const ARROW_LINE: DrawingToolSpec = line_spec(DrawingKind::ArrowLine, 21, "arrow_line");

const fn channel_spec(
    kind: DrawingKind,
    wire_id: u8,
    name: &'static str,
    anchors: u8,
) -> DrawingToolSpec {
    DrawingToolSpec {
        kind,
        wire_id,
        name,
        placement: DrawingPlacement::ClickAnchors { count: anchors },
        straighten: DrawingStraightenMode::None,
        price_extent: DrawingPriceExtent::Full,
        default_width: 1.0,
        ..TREND_LINE
    }
}

const PARALLEL_CHANNEL: DrawingToolSpec =
    channel_spec(DrawingKind::ParallelChannel, 22, "parallel_channel", 3);
const REGRESSION_TREND: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::RegressionTrend, 23, "regression_trend", 2)
};
const FLAT_TOP_CHANNEL: DrawingToolSpec =
    channel_spec(DrawingKind::FlatTopChannel, 24, "flat_top_channel", 3);
const FLAT_BOTTOM_CHANNEL: DrawingToolSpec =
    channel_spec(DrawingKind::FlatBottomChannel, 25, "flat_bottom_channel", 3);
const DISJOINT_CHANNEL: DrawingToolSpec =
    channel_spec(DrawingKind::DisjointChannel, 26, "disjoint_channel", 4);
const POLYLINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Polyline,
    wire_id: 34,
    name: "polyline",
    ..PATH
};
const HIGHLIGHTER: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Highlighter,
    wire_id: 35,
    name: "highlighter",
    default_width: 12.0,
    ..BRUSH
};

const fn shape_spec(
    kind: DrawingKind,
    wire_id: u8,
    name: &'static str,
    count: u8,
) -> DrawingToolSpec {
    DrawingToolSpec {
        kind,
        wire_id,
        name,
        placement: DrawingPlacement::ClickAnchors { count },
        straighten: DrawingStraightenMode::None,
        default_width: 1.0,
        text_layout: DrawingTextLayout::Box,
        ..TREND_LINE
    }
}

const ROTATED_RECTANGLE: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::RotatedRectangle, 27, "rotated_rectangle", 3)
};
const ELLIPSE: DrawingToolSpec = shape_spec(DrawingKind::Ellipse, 28, "ellipse", 2);
const CIRCLE: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::Circle, 29, "circle", 2)
};
const TRIANGLE: DrawingToolSpec = shape_spec(DrawingKind::Triangle, 30, "triangle", 3);
const ARC: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::Arc, 31, "arc", 3)
};
const CURVE: DrawingToolSpec = shape_spec(DrawingKind::Curve, 32, "curve", 3);
const DOUBLE_CURVE: DrawingToolSpec = shape_spec(DrawingKind::DoubleCurve, 33, "double_curve", 4);
const FIBONACCI_RETRACEMENT: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(
        DrawingKind::FibonacciRetracement,
        36,
        "fibonacci_retracement",
        2,
    )
};
const FIBONACCI_EXTENSION: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(
        DrawingKind::FibonacciExtension,
        37,
        "fibonacci_extension",
        3,
    )
};
const FIBONACCI_CHANNEL: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::FibonacciChannel, 38, "fibonacci_channel", 3)
};
const FIBONACCI_TIME_ZONES: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(
        DrawingKind::FibonacciTimeZones,
        39,
        "fibonacci_time_zones",
        2,
    )
};
const FIBONACCI_TREND_TIME: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(
        DrawingKind::FibonacciTrendTime,
        40,
        "fibonacci_trend_time",
        3,
    )
};
const FIBONACCI_SPEED_FAN: DrawingToolSpec = DrawingToolSpec {
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::FibonacciSpeedFan, 41, "fibonacci_speed_fan", 2)
};
const FIBONACCI_SPEED_ARCS: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(
        DrawingKind::FibonacciSpeedArcs,
        42,
        "fibonacci_speed_arcs",
        2,
    )
};
const FIBONACCI_CIRCLES: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::FibonacciCircles, 43, "fibonacci_circles", 2)
};
const FIBONACCI_SPIRAL: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::FibonacciSpiral, 44, "fibonacci_spiral", 2)
};
const FIBONACCI_WEDGE: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::FibonacciWedge, 45, "fibonacci_wedge", 3)
};
const fn pitchfork_spec(kind: DrawingKind, wire_id: u8, name: &'static str) -> DrawingToolSpec {
    DrawingToolSpec {
        logical_extent: DrawingLogicalExtent::Full,
        price_extent: DrawingPriceExtent::Full,
        ..shape_spec(kind, wire_id, name, 3)
    }
}
const ANDREWS_PITCHFORK: DrawingToolSpec =
    pitchfork_spec(DrawingKind::AndrewsPitchfork, 46, "andrews_pitchfork");
const SCHIFF_PITCHFORK: DrawingToolSpec =
    pitchfork_spec(DrawingKind::SchiffPitchfork, 47, "schiff_pitchfork");
const MODIFIED_SCHIFF_PITCHFORK: DrawingToolSpec = pitchfork_spec(
    DrawingKind::ModifiedSchiffPitchfork,
    48,
    "modified_schiff_pitchfork",
);
const INSIDE_PITCHFORK: DrawingToolSpec =
    pitchfork_spec(DrawingKind::InsidePitchfork, 49, "inside_pitchfork");
const PITCHFAN: DrawingToolSpec = pitchfork_spec(DrawingKind::Pitchfan, 50, "pitchfan");
const PATTERN_XABCD: DrawingToolSpec =
    shape_spec(DrawingKind::PatternXabcd, 51, "pattern_xabcd", 5);
const PATTERN_CYPHER: DrawingToolSpec =
    shape_spec(DrawingKind::PatternCypher, 52, "pattern_cypher", 5);
const PATTERN_ABCD: DrawingToolSpec = shape_spec(DrawingKind::PatternAbcd, 53, "pattern_abcd", 4);
const PATTERN_HEAD_SHOULDERS: DrawingToolSpec = shape_spec(
    DrawingKind::PatternHeadShoulders,
    54,
    "pattern_head_shoulders",
    7,
);
const PATTERN_TRIANGLE: DrawingToolSpec =
    shape_spec(DrawingKind::PatternTriangle, 55, "pattern_triangle", 5);
const PATTERN_THREE_DRIVES: DrawingToolSpec = shape_spec(
    DrawingKind::PatternThreeDrives,
    56,
    "pattern_three_drives",
    6,
);
const ELLIOTT_IMPULSE: DrawingToolSpec =
    shape_spec(DrawingKind::ElliottImpulse, 57, "elliott_impulse", 6);
const ELLIOTT_CORRECTION: DrawingToolSpec =
    shape_spec(DrawingKind::ElliottCorrection, 58, "elliott_correction", 4);
const ELLIOTT_TRIANGLE: DrawingToolSpec =
    shape_spec(DrawingKind::ElliottTriangle, 59, "elliott_triangle", 6);
const ELLIOTT_DOUBLE_COMBINATION: DrawingToolSpec = shape_spec(
    DrawingKind::ElliottDoubleCombination,
    60,
    "elliott_double_combination",
    4,
);
const ELLIOTT_TRIPLE_COMBINATION: DrawingToolSpec = shape_spec(
    DrawingKind::ElliottTripleCombination,
    61,
    "elliott_triple_combination",
    6,
);
const CYCLIC_LINES: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::CyclicLines, 62, "cyclic_lines", 2)
};
const TIME_CYCLES: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Ray,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::TimeCycles, 63, "time_cycles", 2)
};
// The wave swings as far below its first anchor as the second sits above it, past the anchors'
// price box, so only its time side bounds it.
const SINE_LINE: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Ray,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::SineLine, 64, "sine_line", 2)
};
const ARROW_MARKER_UP: DrawingToolSpec =
    shape_spec(DrawingKind::ArrowMarkerUp, 65, "arrow_marker_up", 1);
const ARROW_MARKER_DOWN: DrawingToolSpec =
    shape_spec(DrawingKind::ArrowMarkerDown, 66, "arrow_marker_down", 1);
const ARROW_MARKER_LEFT: DrawingToolSpec =
    shape_spec(DrawingKind::ArrowMarkerLeft, 67, "arrow_marker_left", 1);
const ARROW_MARKER_RIGHT: DrawingToolSpec =
    shape_spec(DrawingKind::ArrowMarkerRight, 68, "arrow_marker_right", 1);
const FLAG_MARK: DrawingToolSpec = shape_spec(DrawingKind::FlagMark, 69, "flag_mark", 1);
const SIGNPOST: DrawingToolSpec = shape_spec(DrawingKind::Signpost, 70, "signpost", 2);
const fn text_annotation_spec(
    kind: DrawingKind,
    wire_id: u8,
    name: &'static str,
    count: u8,
) -> DrawingToolSpec {
    DrawingToolSpec {
        kind,
        wire_id,
        name,
        placement: DrawingPlacement::ClickAnchors { count },
        requests_text_editor: true,
        ..TEXT
    }
}
const NOTE: DrawingToolSpec = text_annotation_spec(DrawingKind::Note, 71, "note", 1);
const COMMENT: DrawingToolSpec = text_annotation_spec(DrawingKind::Comment, 72, "comment", 1);
// The tip and the box each keep a handle (the own line's callout editing; upstream's text
// annotations have none), so the leader's tip moves without moving the box.
const CALLOUT: DrawingToolSpec = DrawingToolSpec {
    handles: DrawingHandleMode::Anchors,
    ..text_annotation_spec(DrawingKind::Callout, 73, "callout", 2)
};
// The note's line spans the whole pane width at its price, so no time range bounds it.
const PRICE_NOTE: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..text_annotation_spec(DrawingKind::PriceNote, 74, "price_note", 1)
};
const PRICE_LABEL: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Finite,
    ..shape_spec(DrawingKind::PriceLabel, 75, "price_label", 1)
};
const ANCHORED_TEXT: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..text_annotation_spec(DrawingKind::AnchoredText, 76, "anchored_text", 1)
};
const ICON_STAMP: DrawingToolSpec = shape_spec(DrawingKind::IconStamp, 77, "icon_stamp", 1);
const GANN_BOX: DrawingToolSpec = shape_spec(DrawingKind::GannBox, 78, "gann_box", 2);
const GANN_SQUARE: DrawingToolSpec = shape_spec(DrawingKind::GannSquare, 79, "gann_square", 2);
const GANN_SQUARE_FIXED: DrawingToolSpec =
    shape_spec(DrawingKind::GannSquareFixed, 80, "gann_square_fixed", 2);
const GANN_FAN: DrawingToolSpec = DrawingToolSpec {
    logical_extent: DrawingLogicalExtent::Ray,
    price_extent: DrawingPriceExtent::Full,
    ..shape_spec(DrawingKind::GannFan, 81, "gann_fan", 2)
};
const PROJECTION: DrawingToolSpec = shape_spec(DrawingKind::Projection, 82, "projection", 2);
const FORECAST: DrawingToolSpec = shape_spec(DrawingKind::Forecast, 83, "forecast", 2);
const BARS_PATTERN: DrawingToolSpec = shape_spec(DrawingKind::BarsPattern, 84, "bars_pattern", 3);

/// Every built-in tool: AerisTerminal upstream's contiguous catalog (wire ids `0..=84`, in
/// upstream order), then the own-line tools at the top of the `u8` space (`240..=246`).
pub(crate) const DRAWING_TOOL_SPECS: &[DrawingToolSpec] = &[
    TREND_LINE,
    HORIZONTAL_LINE,
    HORIZONTAL_RAY,
    VERTICAL_LINE,
    RECTANGLE,
    TEXT,
    BRUSH,
    PATH,
    LONG_POSITION,
    SHORT_POSITION,
    FIXED_RANGE_VOLUME_PROFILE,
    ANCHORED_VOLUME_PROFILE,
    ANCHORED_VWAP,
    super::kinds::projection_annotations::PRICE_RANGE,
    super::kinds::projection_annotations::DATE_RANGE,
    super::kinds::projection_annotations::DATE_PRICE_RANGE,
    RAY,
    EXTENDED_LINE,
    INFO_LINE,
    TREND_ANGLE,
    CROSS_LINE,
    ARROW_LINE,
    PARALLEL_CHANNEL,
    REGRESSION_TREND,
    FLAT_TOP_CHANNEL,
    FLAT_BOTTOM_CHANNEL,
    DISJOINT_CHANNEL,
    POLYLINE,
    HIGHLIGHTER,
    ROTATED_RECTANGLE,
    ELLIPSE,
    CIRCLE,
    TRIANGLE,
    ARC,
    CURVE,
    DOUBLE_CURVE,
    FIBONACCI_RETRACEMENT,
    FIBONACCI_EXTENSION,
    FIBONACCI_CHANNEL,
    FIBONACCI_TIME_ZONES,
    FIBONACCI_TREND_TIME,
    FIBONACCI_SPEED_FAN,
    FIBONACCI_SPEED_ARCS,
    FIBONACCI_CIRCLES,
    FIBONACCI_SPIRAL,
    FIBONACCI_WEDGE,
    ANDREWS_PITCHFORK,
    SCHIFF_PITCHFORK,
    MODIFIED_SCHIFF_PITCHFORK,
    INSIDE_PITCHFORK,
    PITCHFAN,
    PATTERN_XABCD,
    PATTERN_CYPHER,
    PATTERN_ABCD,
    PATTERN_HEAD_SHOULDERS,
    PATTERN_TRIANGLE,
    PATTERN_THREE_DRIVES,
    ELLIOTT_IMPULSE,
    ELLIOTT_CORRECTION,
    ELLIOTT_TRIANGLE,
    ELLIOTT_DOUBLE_COMBINATION,
    ELLIOTT_TRIPLE_COMBINATION,
    CYCLIC_LINES,
    TIME_CYCLES,
    SINE_LINE,
    ARROW_MARKER_UP,
    ARROW_MARKER_DOWN,
    ARROW_MARKER_LEFT,
    ARROW_MARKER_RIGHT,
    FLAG_MARK,
    SIGNPOST,
    NOTE,
    COMMENT,
    CALLOUT,
    PRICE_NOTE,
    PRICE_LABEL,
    ANCHORED_TEXT,
    ICON_STAMP,
    GANN_BOX,
    GANN_SQUARE,
    GANN_SQUARE_FIXED,
    GANN_FAN,
    PROJECTION,
    FORECAST,
    BARS_PATTERN,
    // Own-line tools (not in AerisTerminal upstream), wire ids 240..=246.
    super::kinds::lines::HORIZONTAL_SEGMENT,
    super::kinds::lines::VERTICAL_RAY,
    super::kinds::lines::VERTICAL_SEGMENT,
    super::kinds::lines::PRICE_LINE,
    super::kinds::channels::PRICE_CHANNEL,
    super::kinds::projection_annotations::SIMPLE_TAG,
    super::kinds::projection_annotations::SIMPLE_ANNOTATION,
];

impl DrawingKind {
    /// The measuring tools (price, date, and date-and-price range).
    pub(crate) const fn is_measure(self) -> bool {
        matches!(
            self,
            Self::PriceRange | Self::DateRange | Self::DatePriceRange
        )
    }

    pub(crate) const fn spec(self) -> &'static DrawingToolSpec {
        match self {
            Self::TrendLine => &TREND_LINE,
            Self::HorizontalLine => &HORIZONTAL_LINE,
            Self::HorizontalRay => &HORIZONTAL_RAY,
            Self::VerticalLine => &VERTICAL_LINE,
            Self::Rectangle => &RECTANGLE,
            Self::Text => &TEXT,
            Self::Brush => &BRUSH,
            Self::Path => &PATH,
            Self::LongPosition => &LONG_POSITION,
            Self::ShortPosition => &SHORT_POSITION,
            Self::FixedRangeVolumeProfile => &FIXED_RANGE_VOLUME_PROFILE,
            Self::AnchoredVolumeProfile => &ANCHORED_VOLUME_PROFILE,
            Self::AnchoredVwap => &ANCHORED_VWAP,
            Self::PriceRange => &super::kinds::projection_annotations::PRICE_RANGE,
            Self::DateRange => &super::kinds::projection_annotations::DATE_RANGE,
            Self::DatePriceRange => &super::kinds::projection_annotations::DATE_PRICE_RANGE,
            Self::Ray => &RAY,
            Self::ExtendedLine => &EXTENDED_LINE,
            Self::InfoLine => &INFO_LINE,
            Self::TrendAngle => &TREND_ANGLE,
            Self::CrossLine => &CROSS_LINE,
            Self::ArrowLine => &ARROW_LINE,
            Self::ParallelChannel => &PARALLEL_CHANNEL,
            Self::RegressionTrend => &REGRESSION_TREND,
            Self::FlatTopChannel => &FLAT_TOP_CHANNEL,
            Self::FlatBottomChannel => &FLAT_BOTTOM_CHANNEL,
            Self::DisjointChannel => &DISJOINT_CHANNEL,
            Self::Polyline => &POLYLINE,
            Self::Highlighter => &HIGHLIGHTER,
            Self::RotatedRectangle => &ROTATED_RECTANGLE,
            Self::Ellipse => &ELLIPSE,
            Self::Circle => &CIRCLE,
            Self::Triangle => &TRIANGLE,
            Self::Arc => &ARC,
            Self::Curve => &CURVE,
            Self::DoubleCurve => &DOUBLE_CURVE,
            Self::FibonacciRetracement => &FIBONACCI_RETRACEMENT,
            Self::FibonacciExtension => &FIBONACCI_EXTENSION,
            Self::FibonacciChannel => &FIBONACCI_CHANNEL,
            Self::FibonacciTimeZones => &FIBONACCI_TIME_ZONES,
            Self::FibonacciTrendTime => &FIBONACCI_TREND_TIME,
            Self::FibonacciSpeedFan => &FIBONACCI_SPEED_FAN,
            Self::FibonacciSpeedArcs => &FIBONACCI_SPEED_ARCS,
            Self::FibonacciCircles => &FIBONACCI_CIRCLES,
            Self::FibonacciSpiral => &FIBONACCI_SPIRAL,
            Self::FibonacciWedge => &FIBONACCI_WEDGE,
            Self::AndrewsPitchfork => &ANDREWS_PITCHFORK,
            Self::SchiffPitchfork => &SCHIFF_PITCHFORK,
            Self::ModifiedSchiffPitchfork => &MODIFIED_SCHIFF_PITCHFORK,
            Self::InsidePitchfork => &INSIDE_PITCHFORK,
            Self::Pitchfan => &PITCHFAN,
            Self::PatternXabcd => &PATTERN_XABCD,
            Self::PatternCypher => &PATTERN_CYPHER,
            Self::PatternAbcd => &PATTERN_ABCD,
            Self::PatternHeadShoulders => &PATTERN_HEAD_SHOULDERS,
            Self::PatternTriangle => &PATTERN_TRIANGLE,
            Self::PatternThreeDrives => &PATTERN_THREE_DRIVES,
            Self::ElliottImpulse => &ELLIOTT_IMPULSE,
            Self::ElliottCorrection => &ELLIOTT_CORRECTION,
            Self::ElliottTriangle => &ELLIOTT_TRIANGLE,
            Self::ElliottDoubleCombination => &ELLIOTT_DOUBLE_COMBINATION,
            Self::ElliottTripleCombination => &ELLIOTT_TRIPLE_COMBINATION,
            Self::CyclicLines => &CYCLIC_LINES,
            Self::TimeCycles => &TIME_CYCLES,
            Self::SineLine => &SINE_LINE,
            Self::ArrowMarkerUp => &ARROW_MARKER_UP,
            Self::ArrowMarkerDown => &ARROW_MARKER_DOWN,
            Self::ArrowMarkerLeft => &ARROW_MARKER_LEFT,
            Self::ArrowMarkerRight => &ARROW_MARKER_RIGHT,
            Self::FlagMark => &FLAG_MARK,
            Self::Signpost => &SIGNPOST,
            Self::Note => &NOTE,
            Self::Comment => &COMMENT,
            Self::Callout => &CALLOUT,
            Self::PriceNote => &PRICE_NOTE,
            Self::PriceLabel => &PRICE_LABEL,
            Self::AnchoredText => &ANCHORED_TEXT,
            Self::IconStamp => &ICON_STAMP,
            Self::GannBox => &GANN_BOX,
            Self::GannSquare => &GANN_SQUARE,
            Self::GannSquareFixed => &GANN_SQUARE_FIXED,
            Self::GannFan => &GANN_FAN,
            Self::Projection => &PROJECTION,
            Self::Forecast => &FORECAST,
            Self::BarsPattern => &BARS_PATTERN,
            Self::HorizontalSegment => &super::kinds::lines::HORIZONTAL_SEGMENT,
            Self::VerticalRay => &super::kinds::lines::VERTICAL_RAY,
            Self::VerticalSegment => &super::kinds::lines::VERTICAL_SEGMENT,
            Self::PriceLine => &super::kinds::lines::PRICE_LINE,
            Self::PriceChannel => &super::kinds::channels::PRICE_CHANNEL,
            Self::SimpleTag => &super::kinds::projection_annotations::SIMPLE_TAG,
            Self::SimpleAnnotation => &super::kinds::projection_annotations::SIMPLE_ANNOTATION,
        }
    }

    pub(crate) const fn vertex_labels(self) -> Option<&'static [&'static str]> {
        match self {
            Self::PatternXabcd | Self::PatternCypher => Some(&["X", "A", "B", "C", "D"]),
            Self::PatternAbcd => Some(&["A", "B", "C", "D"]),
            Self::PatternHeadShoulders => Some(&["N", "LS", "N", "H", "N", "RS", "N"]),
            Self::PatternTriangle => Some(&["A", "B", "C", "D", "E"]),
            Self::PatternThreeDrives => Some(&["0", "1", "A", "2", "B", "3"]),
            Self::ElliottImpulse => Some(&["0", "1", "2", "3", "4", "5"]),
            Self::ElliottCorrection => Some(&["0", "A", "B", "C"]),
            Self::ElliottTriangle => Some(&["0", "A", "B", "C", "D", "E"]),
            Self::ElliottDoubleCombination => Some(&["0", "W", "X", "Y"]),
            Self::ElliottTripleCombination => Some(&["0", "W", "X", "Y", "X", "Z"]),
            _ => None,
        }
    }

    pub(crate) const fn is_elliott(self) -> bool {
        matches!(
            self,
            Self::ElliottImpulse
                | Self::ElliottCorrection
                | Self::ElliottTriangle
                | Self::ElliottDoubleCombination
                | Self::ElliottTripleCombination
        )
    }

    pub(crate) const fn is_marker(self) -> bool {
        matches!(
            self,
            Self::ArrowMarkerUp
                | Self::ArrowMarkerDown
                | Self::ArrowMarkerLeft
                | Self::ArrowMarkerRight
                | Self::FlagMark
                | Self::Signpost
        )
    }

    pub(crate) const fn is_text_annotation(self) -> bool {
        matches!(
            self,
            Self::Note | Self::Comment | Self::Callout | Self::PriceNote | Self::AnchoredText
        )
    }

    pub(crate) fn valid_wave_degree(name: &str) -> bool {
        matches!(
            name,
            "subminuette"
                | "minuette"
                | "minute"
                | "minor"
                | "intermediate"
                | "primary"
                | "cycle"
                | "supercycle"
                | "grand_supercycle"
                | "submillennium"
                | "millennium"
                | "supermillennium"
        )
    }
}
