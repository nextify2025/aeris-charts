//! Compile-time drawing-tool catalog.
//!
//! Tool semantics belong here rather than in browser/native hosts.  The catalog is deliberately
//! static: Aeris needs one deterministic implementation shared by every backend, not a runtime
//! plugin registry.  New built-in tools should describe their placement/editing invariants here
//! and keep only genuinely tool-specific geometry/math in the drawing engine.
//!
//! Wire ids are reserved per family so parallel family work never collides:
//! core 0..=31, lines 32..=47, channels 48..=63, fibonacci 64..=95, pitchforks_gann 96..=127,
//! projection_annotations 128..=159, patterns_elliott_cycles 160..=191, shapes 192..=223, and
//! 224..=255 unassigned. B8 family tools define their `DrawingToolSpec` constants in their own
//! `kinds/<family>.rs` module; this file lists them inside that family's reserved block.

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
};

/// Every built-in tool. B8 families append only inside their own reserved block.
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
    // B8: lines — begin
    super::kinds::lines::RAY,
    super::kinds::lines::EXTENDED_LINE,
    super::kinds::lines::INFO_LINE,
    super::kinds::lines::TREND_ANGLE,
    super::kinds::lines::CROSS_LINE,
    super::kinds::lines::ARROW_LINE,
    super::kinds::lines::HORIZONTAL_SEGMENT,
    super::kinds::lines::VERTICAL_RAY,
    super::kinds::lines::VERTICAL_SEGMENT,
    // B8: lines — end
    // B8: channels — begin
    super::kinds::channels::PARALLEL_CHANNEL,
    super::kinds::channels::REGRESSION_TREND,
    super::kinds::channels::FLAT_TOP_BOTTOM,
    super::kinds::channels::DISJOINT_CHANNEL,
    super::kinds::channels::PRICE_CHANNEL,
    // B8: channels — end
    // B8: fibonacci — begin
    super::kinds::fibonacci::FIB_RETRACEMENT,
    super::kinds::fibonacci::TREND_BASED_FIB_EXTENSION,
    super::kinds::fibonacci::FIB_CHANNEL,
    super::kinds::fibonacci::FIB_TIME_ZONE,
    super::kinds::fibonacci::TREND_BASED_FIB_TIME,
    super::kinds::fibonacci::FIB_SPEED_RESISTANCE_FAN,
    super::kinds::fibonacci::FIB_SPEED_RESISTANCE_ARCS,
    super::kinds::fibonacci::FIB_CIRCLES,
    super::kinds::fibonacci::FIB_SPIRAL,
    super::kinds::fibonacci::FIB_WEDGE,
    // B8: fibonacci — end
    // B8: pitchforks_gann — begin
    super::kinds::pitchforks_gann::ANDREWS_PITCHFORK,
    super::kinds::pitchforks_gann::SCHIFF_PITCHFORK,
    super::kinds::pitchforks_gann::MODIFIED_SCHIFF_PITCHFORK,
    super::kinds::pitchforks_gann::INSIDE_PITCHFORK,
    super::kinds::pitchforks_gann::PITCHFAN,
    super::kinds::pitchforks_gann::GANN_BOX,
    super::kinds::pitchforks_gann::GANN_SQUARE,
    super::kinds::pitchforks_gann::GANN_SQUARE_FIXED,
    super::kinds::pitchforks_gann::GANN_FAN,
    // B8: pitchforks_gann — end
    // B8: projection_annotations — begin
    super::kinds::projection_annotations::FORECAST,
    super::kinds::projection_annotations::BARS_PATTERN,
    super::kinds::projection_annotations::PRICE_RANGE,
    super::kinds::projection_annotations::DATE_RANGE,
    super::kinds::projection_annotations::DATE_AND_PRICE_RANGE,
    super::kinds::projection_annotations::PROJECTION,
    super::kinds::projection_annotations::ANCHORED_TEXT,
    super::kinds::projection_annotations::NOTE,
    super::kinds::projection_annotations::PRICE_NOTE,
    super::kinds::projection_annotations::CALLOUT,
    super::kinds::projection_annotations::COMMENT,
    super::kinds::projection_annotations::PRICE_LABEL,
    super::kinds::projection_annotations::SIGNPOST,
    super::kinds::projection_annotations::FLAG_MARK,
    super::kinds::projection_annotations::ARROW_MARK_UP,
    super::kinds::projection_annotations::ARROW_MARK_DOWN,
    super::kinds::projection_annotations::ARROW_MARK_LEFT,
    super::kinds::projection_annotations::ARROW_MARK_RIGHT,
    super::kinds::projection_annotations::ICON,
    // B8: projection_annotations — end
    // B8: patterns_elliott_cycles — begin
    super::kinds::patterns_elliott_cycles::XABCD_PATTERN,
    super::kinds::patterns_elliott_cycles::CYPHER_PATTERN,
    super::kinds::patterns_elliott_cycles::ABCD_PATTERN,
    super::kinds::patterns_elliott_cycles::HEAD_AND_SHOULDERS,
    super::kinds::patterns_elliott_cycles::TRIANGLE_PATTERN,
    super::kinds::patterns_elliott_cycles::THREE_DRIVES_PATTERN,
    super::kinds::patterns_elliott_cycles::ELLIOTT_IMPULSE_WAVE,
    super::kinds::patterns_elliott_cycles::ELLIOTT_CORRECTION_WAVE,
    super::kinds::patterns_elliott_cycles::ELLIOTT_TRIANGLE_WAVE,
    super::kinds::patterns_elliott_cycles::ELLIOTT_DOUBLE_COMBO,
    super::kinds::patterns_elliott_cycles::ELLIOTT_TRIPLE_COMBO,
    super::kinds::patterns_elliott_cycles::CYCLIC_LINES,
    super::kinds::patterns_elliott_cycles::TIME_CYCLES,
    super::kinds::patterns_elliott_cycles::SINE_LINE,
    // B8: patterns_elliott_cycles — end
    // B8: shapes — begin
    super::kinds::shapes::ROTATED_RECTANGLE,
    super::kinds::shapes::ELLIPSE,
    super::kinds::shapes::CIRCLE,
    super::kinds::shapes::TRIANGLE,
    super::kinds::shapes::ARC,
    super::kinds::shapes::CURVE,
    super::kinds::shapes::DOUBLE_CURVE,
    super::kinds::shapes::POLYLINE,
    super::kinds::shapes::HIGHLIGHTER,
    // B8: shapes — end
];

impl DrawingKind {
    /// The measuring tools (price, date, and date-and-price range).
    pub(crate) const fn is_measure(self) -> bool {
        matches!(
            self,
            Self::PriceRange | Self::DateRange | Self::DateAndPriceRange
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
            // B8: lines — begin
            Self::Ray => &super::kinds::lines::RAY,
            Self::ExtendedLine => &super::kinds::lines::EXTENDED_LINE,
            Self::InfoLine => &super::kinds::lines::INFO_LINE,
            Self::TrendAngle => &super::kinds::lines::TREND_ANGLE,
            Self::CrossLine => &super::kinds::lines::CROSS_LINE,
            Self::ArrowLine => &super::kinds::lines::ARROW_LINE,
            Self::HorizontalSegment => &super::kinds::lines::HORIZONTAL_SEGMENT,
            Self::VerticalRay => &super::kinds::lines::VERTICAL_RAY,
            Self::VerticalSegment => &super::kinds::lines::VERTICAL_SEGMENT,
            // B8: lines — end
            // B8: channels — begin
            Self::ParallelChannel => &super::kinds::channels::PARALLEL_CHANNEL,
            Self::RegressionTrend => &super::kinds::channels::REGRESSION_TREND,
            Self::FlatTopBottom => &super::kinds::channels::FLAT_TOP_BOTTOM,
            Self::DisjointChannel => &super::kinds::channels::DISJOINT_CHANNEL,
            Self::PriceChannel => &super::kinds::channels::PRICE_CHANNEL,
            // B8: channels — end
            // B8: fibonacci — begin
            Self::FibRetracement => &super::kinds::fibonacci::FIB_RETRACEMENT,
            Self::TrendBasedFibExtension => &super::kinds::fibonacci::TREND_BASED_FIB_EXTENSION,
            Self::FibChannel => &super::kinds::fibonacci::FIB_CHANNEL,
            Self::FibTimeZone => &super::kinds::fibonacci::FIB_TIME_ZONE,
            Self::TrendBasedFibTime => &super::kinds::fibonacci::TREND_BASED_FIB_TIME,
            Self::FibSpeedResistanceFan => &super::kinds::fibonacci::FIB_SPEED_RESISTANCE_FAN,
            Self::FibSpeedResistanceArcs => &super::kinds::fibonacci::FIB_SPEED_RESISTANCE_ARCS,
            Self::FibCircles => &super::kinds::fibonacci::FIB_CIRCLES,
            Self::FibSpiral => &super::kinds::fibonacci::FIB_SPIRAL,
            Self::FibWedge => &super::kinds::fibonacci::FIB_WEDGE,
            // B8: fibonacci — end
            // B8: pitchforks_gann — begin
            Self::AndrewsPitchfork => &super::kinds::pitchforks_gann::ANDREWS_PITCHFORK,
            Self::SchiffPitchfork => &super::kinds::pitchforks_gann::SCHIFF_PITCHFORK,
            Self::ModifiedSchiffPitchfork => {
                &super::kinds::pitchforks_gann::MODIFIED_SCHIFF_PITCHFORK
            }
            Self::InsidePitchfork => &super::kinds::pitchforks_gann::INSIDE_PITCHFORK,
            Self::Pitchfan => &super::kinds::pitchforks_gann::PITCHFAN,
            Self::GannBox => &super::kinds::pitchforks_gann::GANN_BOX,
            Self::GannSquare => &super::kinds::pitchforks_gann::GANN_SQUARE,
            Self::GannSquareFixed => &super::kinds::pitchforks_gann::GANN_SQUARE_FIXED,
            Self::GannFan => &super::kinds::pitchforks_gann::GANN_FAN,
            // B8: pitchforks_gann — end
            // B8: projection_annotations — begin
            Self::Forecast => &super::kinds::projection_annotations::FORECAST,
            Self::BarsPattern => &super::kinds::projection_annotations::BARS_PATTERN,
            Self::PriceRange => &super::kinds::projection_annotations::PRICE_RANGE,
            Self::DateRange => &super::kinds::projection_annotations::DATE_RANGE,
            Self::DateAndPriceRange => &super::kinds::projection_annotations::DATE_AND_PRICE_RANGE,
            Self::Projection => &super::kinds::projection_annotations::PROJECTION,
            Self::AnchoredText => &super::kinds::projection_annotations::ANCHORED_TEXT,
            Self::Note => &super::kinds::projection_annotations::NOTE,
            Self::PriceNote => &super::kinds::projection_annotations::PRICE_NOTE,
            Self::Callout => &super::kinds::projection_annotations::CALLOUT,
            Self::Comment => &super::kinds::projection_annotations::COMMENT,
            Self::PriceLabel => &super::kinds::projection_annotations::PRICE_LABEL,
            Self::Signpost => &super::kinds::projection_annotations::SIGNPOST,
            Self::FlagMark => &super::kinds::projection_annotations::FLAG_MARK,
            Self::ArrowMarkUp => &super::kinds::projection_annotations::ARROW_MARK_UP,
            Self::ArrowMarkDown => &super::kinds::projection_annotations::ARROW_MARK_DOWN,
            Self::ArrowMarkLeft => &super::kinds::projection_annotations::ARROW_MARK_LEFT,
            Self::ArrowMarkRight => &super::kinds::projection_annotations::ARROW_MARK_RIGHT,
            Self::Icon => &super::kinds::projection_annotations::ICON,
            // B8: projection_annotations — end
            // B8: patterns_elliott_cycles — begin
            Self::XabcdPattern => &super::kinds::patterns_elliott_cycles::XABCD_PATTERN,
            Self::CypherPattern => &super::kinds::patterns_elliott_cycles::CYPHER_PATTERN,
            Self::AbcdPattern => &super::kinds::patterns_elliott_cycles::ABCD_PATTERN,
            Self::HeadAndShoulders => &super::kinds::patterns_elliott_cycles::HEAD_AND_SHOULDERS,
            Self::TrianglePattern => &super::kinds::patterns_elliott_cycles::TRIANGLE_PATTERN,
            Self::ThreeDrivesPattern => {
                &super::kinds::patterns_elliott_cycles::THREE_DRIVES_PATTERN
            }
            Self::ElliottImpulseWave => {
                &super::kinds::patterns_elliott_cycles::ELLIOTT_IMPULSE_WAVE
            }
            Self::ElliottCorrectionWave => {
                &super::kinds::patterns_elliott_cycles::ELLIOTT_CORRECTION_WAVE
            }
            Self::ElliottTriangleWave => {
                &super::kinds::patterns_elliott_cycles::ELLIOTT_TRIANGLE_WAVE
            }
            Self::ElliottDoubleCombo => {
                &super::kinds::patterns_elliott_cycles::ELLIOTT_DOUBLE_COMBO
            }
            Self::ElliottTripleCombo => {
                &super::kinds::patterns_elliott_cycles::ELLIOTT_TRIPLE_COMBO
            }
            Self::CyclicLines => &super::kinds::patterns_elliott_cycles::CYCLIC_LINES,
            Self::TimeCycles => &super::kinds::patterns_elliott_cycles::TIME_CYCLES,
            Self::SineLine => &super::kinds::patterns_elliott_cycles::SINE_LINE,
            // B8: patterns_elliott_cycles — end
            // B8: shapes — begin
            Self::RotatedRectangle => &super::kinds::shapes::ROTATED_RECTANGLE,
            Self::Ellipse => &super::kinds::shapes::ELLIPSE,
            Self::Circle => &super::kinds::shapes::CIRCLE,
            Self::Triangle => &super::kinds::shapes::TRIANGLE,
            Self::Arc => &super::kinds::shapes::ARC,
            Self::Curve => &super::kinds::shapes::CURVE,
            Self::DoubleCurve => &super::kinds::shapes::DOUBLE_CURVE,
            Self::Polyline => &super::kinds::shapes::POLYLINE,
            Self::Highlighter => &super::kinds::shapes::HIGHLIGHTER,
            // B8: shapes — end
        }
    }
}
