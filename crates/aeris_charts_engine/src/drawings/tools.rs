//! Compile-time drawing-tool catalog.
//!
//! Tool semantics belong here rather than in browser/native hosts.  The catalog is deliberately
//! static: Aeris needs one deterministic implementation shared by every backend, not a runtime
//! plugin registry.  New built-in tools should describe their placement/editing invariants here
//! and keep only genuinely tool-specific geometry/math in the drawing engine.

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingPriceExtent {
    Finite,
    Full,
}

/// A coordinate every anchor of the drawing shares. Placing or dragging one anchor moves that
/// coordinate on all of them, the way KLineChart's segment overlays keep themselves straight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawingAnchorLink {
    None,
    /// All anchors share one price (a horizontal segment).
    SamePrice,
    /// All anchors share one logical index (a vertical ray or segment).
    SameLogical,
}

impl DrawingAnchorLink {
    /// Give every point the linked coordinate of `points[source]`.
    pub(crate) fn apply(self, points: &mut [super::DrawingPoint], source: usize) {
        let Some(&source) = points.get(source) else {
            return;
        };
        for point in points {
            match self {
                Self::None => {}
                Self::SamePrice => point.price = source.price,
                Self::SameLogical => point.logical = source.logical,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub(crate) anchor_link: DrawingAnchorLink,
    /// Anchors (placed plus the live cursor) needed before the creation preview draws the shape.
    /// Three-anchor channels already draw their first line from two.
    pub(crate) preview_points: u8,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 3,
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
    anchor_link: DrawingAnchorLink::None,
    preview_points: 3,
};

// KLineChart's overlays (`src/extension/overlay`) that Aeris had no tool for. Wire ids continue
// the existing sequence so older documents keep their meaning.

/// A line through two anchors, extended across the whole pane (KLineChart `straightLine`).
const STRAIGHT_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::StraightLine,
    wire_id: 10,
    name: "straight_line",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
};

/// A line from the first anchor through the second to the pane edge (KLineChart `rayLine`).
const RAY_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::RayLine,
    wire_id: 11,
    name: "ray_line",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
};

/// A horizontal segment between two anchors at one price (KLineChart `horizontalSegment`).
const HORIZONTAL_SEGMENT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::HorizontalSegment,
    wire_id: 12,
    name: "horizontal_segment",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::SamePrice,
    preview_points: 2,
};

/// A vertical line from the first anchor to the pane edge on the second anchor's side
/// (KLineChart `verticalRayLine`).
const VERTICAL_RAY: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::VerticalRay,
    wire_id: 13,
    name: "vertical_ray",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::SameLogical,
    preview_points: 2,
};

/// A vertical segment between two anchors on one bar (KLineChart `verticalSegment`).
const VERTICAL_SEGMENT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::VerticalSegment,
    wire_id: 14,
    name: "vertical_segment",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::SameLogical,
    preview_points: 2,
};

/// A ray to the right from one anchor, labeled with the anchor's price (KLineChart `priceLine`).
const PRICE_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceLine,
    wire_id: 15,
    name: "price_line",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::FromFirst,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
};

/// Two parallel lines: one through the first two anchors, one through the third
/// (KLineChart `parallelStraightLine`).
const PARALLEL_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ParallelLine,
    wire_id: 16,
    name: "parallel_line",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
};

/// A price channel: the parallel pair plus a third line mirrored on the far side of the first
/// (KLineChart `priceChannelLine`).
const PRICE_CHANNEL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceChannel,
    wire_id: 17,
    name: "price_channel",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
};

/// Fibonacci retracement levels between two anchors' prices (KLineChart `fibonacciLine`).
const FIBONACCI_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FibonacciLine,
    wire_id: 18,
    name: "fibonacci_line",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 2,
};

/// A callout: a dashed stem and arrow above one anchor with the drawing's text on top
/// (KLineChart `simpleAnnotation`).
const SIMPLE_ANNOTATION: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::SimpleAnnotation,
    wire_id: 19,
    name: "simple_annotation",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
};

/// A dashed full-width line whose price-axis tag shows the drawing's text, or its price when the
/// text is empty (KLineChart `simpleTag`).
const SIMPLE_TAG: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::SimpleTag,
    wire_id: 20,
    name: "simple_tag",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::VerticalOnly,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    anchor_link: DrawingAnchorLink::None,
    preview_points: 1,
};

pub(crate) const DRAWING_TOOL_SPECS: [DrawingToolSpec; 21] = [
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
    STRAIGHT_LINE,
    RAY_LINE,
    HORIZONTAL_SEGMENT,
    VERTICAL_RAY,
    VERTICAL_SEGMENT,
    PRICE_LINE,
    PARALLEL_LINE,
    PRICE_CHANNEL,
    FIBONACCI_LINE,
    SIMPLE_ANNOTATION,
    SIMPLE_TAG,
];

impl DrawingKind {
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
            Self::StraightLine => &STRAIGHT_LINE,
            Self::RayLine => &RAY_LINE,
            Self::HorizontalSegment => &HORIZONTAL_SEGMENT,
            Self::VerticalRay => &VERTICAL_RAY,
            Self::VerticalSegment => &VERTICAL_SEGMENT,
            Self::PriceLine => &PRICE_LINE,
            Self::ParallelLine => &PARALLEL_LINE,
            Self::PriceChannel => &PRICE_CHANNEL,
            Self::FibonacciLine => &FIBONACCI_LINE,
            Self::SimpleAnnotation => &SIMPLE_ANNOTATION,
            Self::SimpleTag => &SIMPLE_TAG,
        }
    }
}
