//! Typed drawing customization contracts shared by native and browser hosts.
//!
//! The engine keeps the live [`Drawing`](crate::Drawing) representation compact for the hot
//! render and hit-test paths.  This module is the stable, typed view used by property panels,
//! templates, persistence adapters, and cross-cell synchronization.  Values are deliberately
//! bounded and deterministic so a host cannot turn a drawing patch into unbounded work.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;

use crate::{ChartError, DrawingAnchor, DrawingKind, DrawingPriceScale, ErrorCode};

pub const DRAWING_CONTRACT_REVISION: u32 = 1;
pub const MAX_DRAWING_NAME_BYTES: usize = 256;
pub const MAX_DRAWING_GROUP_BYTES: usize = 128;
pub const MAX_DRAWING_LABELS: usize = 32;
pub const MAX_DRAWING_LEVELS: usize = 64;
pub const MAX_DRAWING_TEMPLATE_BYTES: usize = 64 * 1024;
pub const MAX_DRAWING_TEMPLATES: usize = 128;
pub const MAX_DRAWING_OBJECTS: usize = 10_000;
/// Byte bound of a drawing clipboard payload. It carries the same drawing records a persisted
/// document does, so it takes that document's bound rather than the (much smaller) template one.
pub const MAX_DRAWING_CLIPBOARD_BYTES: usize = crate::PERSISTENCE_MAX_DOCUMENT_BYTES;
/// Anchor bound of a drawing clipboard payload across all its drawings, the persisted document's.
pub const MAX_DRAWING_CLIPBOARD_POINTS: usize = crate::PERSISTENCE_MAX_TOTAL_POINTS;
/// Upper bound on the segments one price-basis rescale may carry.
pub const MAX_DRAWING_PRICE_SEGMENTS: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingPropertyType {
    Boolean,
    Number,
    Integer,
    String,
    Color,
    Enum,
    Points,
    Levels,
    IntervalSet,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingPropertyDescriptor {
    pub name: String,
    pub property_type: DrawingPropertyType,
    pub default: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingPropertySchema {
    pub revision: u32,
    pub kind: DrawingKind,
    pub properties: Vec<DrawingPropertyDescriptor>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingLineCap {
    #[default]
    None,
    Arrow,
    Circle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingMagnetMode {
    #[default]
    Off,
    Weak,
    Strong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingIntervalUnit {
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Ticks,
    Ranges,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingInterval {
    pub unit: DrawingIntervalUnit,
    pub value: f64,
}

impl DrawingInterval {
    pub fn validate(self) -> bool {
        self.value.is_finite() && self.value > 0.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingIntervalVisibility {
    pub enabled: bool,
    #[serde(default)]
    pub intervals: Vec<DrawingInterval>,
}

impl DrawingIntervalVisibility {
    pub fn allows(&self, current: Option<DrawingInterval>) -> bool {
        if !self.enabled {
            return true;
        }
        let Some(current) = current else {
            return false;
        };
        self.intervals.iter().any(|allowed| {
            allowed.unit == current.unit
                && (allowed.value - current.value).abs() <= f64::EPSILON.max(current.value * 1e-9)
        })
    }

    pub fn validate(&self) -> bool {
        self.intervals.len() <= 16 && self.intervals.iter().all(|interval| interval.validate())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingLabelMetric {
    Price,
    PriceChange,
    PercentChange,
    Ticks,
    BarCount,
    DateTimeRange,
    Duration,
    Angle,
    Distance,
    VolumeInRange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingLabelPosition {
    Above,
    #[default]
    On,
    Below,
    Inside,
    Outside,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingLabelOptions {
    pub metric: DrawingLabelMetric,
    pub visible: bool,
    pub position: DrawingLabelPosition,
    #[serde(default)]
    pub text: Option<String>,
}

impl DrawingLabelOptions {
    pub fn validate(&self) -> bool {
        self.text.as_ref().is_none_or(|text| text.len() <= 256)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingLevel {
    pub value: f64,
    pub color: String,
    pub visible: bool,
    pub style: String,
    pub fill_between: bool,
    #[serde(default)]
    pub fill_color: Option<String>,
    pub label_visible: bool,
}

impl DrawingLevel {
    pub fn validate(&self) -> bool {
        self.value.is_finite()
            && self.color.len() <= 256
            && self
                .fill_color
                .as_ref()
                .is_none_or(|color| color.len() <= 256)
    }

    /// A visible solid level at `value` in `color` with its label shown and no fill.
    pub fn at(value: f64, color: &str) -> Self {
        Self {
            value,
            color: color.to_string(),
            visible: true,
            style: "solid".to_string(),
            fill_between: false,
            fill_color: None,
            label_visible: true,
        }
    }

    /// The level's price between two anchor prices: `start` at value 0, `end` at value 1, and
    /// linear beyond them (extension levels above 1 or below 0).
    pub fn price_between(&self, start: f64, end: f64) -> f64 {
        start + (end - start) * self.value
    }

    /// Level label text: the ratio (`0.618`), the formatted price, or both (`0.618 (123.45)`).
    pub fn label(&self, show_value: bool, price_text: Option<&str>) -> String {
        match (show_value, price_text) {
            (true, Some(price)) => format!("{} ({price})", self.value),
            (true, None) => self.value.to_string(),
            (false, Some(price)) => price.to_string(),
            (false, None) => String::new(),
        }
    }

    /// The level's stroke color, or `drawing`'s when it does not parse.
    pub(crate) fn stroke_color(&self, drawing: &crate::Drawing) -> Color {
        Color::parse_css(&self.color).unwrap_or_else(|| drawing.stroke_color())
    }

    /// The level's line style (the drawing style names, retired ones folded).
    pub(crate) fn line_style(&self) -> LineStyle {
        crate::drawings::line_style_from_name(&self.style)
    }

    /// The level's zone fill: `fill_color`, else its stroke color at 20% of that color's alpha.
    pub(crate) fn zone_fill(&self, drawing: &crate::Drawing) -> Color {
        self.fill_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or_else(|| {
                let color = self.stroke_color(drawing);
                let alpha = u16::from(color.a()) * 51 / 255;
                Color::rgba(color.r(), color.g(), color.b(), alpha as u8)
            })
    }
}

/// Standard Fibonacci retracement/extension ratios, as level values between two anchors.
pub const FIBONACCI_RATIOS: [f64; 11] = [
    0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0, 1.618, 2.618, 3.618, 4.236,
];

/// Standard Fibonacci time-zone offsets, in bars from the first anchor.
pub const FIBONACCI_TIME_ZONES: [u32; 11] = [0, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89];

/// One visible level per ratio, all in `color` (the default level list of a ratio table).
pub fn drawing_levels_from_ratios(ratios: &[f64], color: &str) -> Vec<DrawingLevel> {
    ratios
        .iter()
        .map(|&value| DrawingLevel::at(value, color))
        .collect()
}

/// One drawing in a clipboard or sync payload. Each point carries its anchor time identity, so a
/// receiving chart with a different interval or history window resolves it by time.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingClipboardItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub kind: DrawingKind,
    pub pane_index: usize,
    pub points: Vec<DrawingAnchor>,
    pub options: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingClipboardPayload {
    pub schema: String,
    pub revision: u64,
    pub drawings: Vec<DrawingClipboardItem>,
    /// Host-defined price basis the copied prices use (see
    /// [`ChartEngine::set_drawing_price_basis`](crate::ChartEngine::set_drawing_price_basis)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_basis: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingSyncPayload {
    pub schema: String,
    pub source: String,
    pub revision: u64,
    pub drawings: Vec<DrawingClipboardItem>,
    /// Host-defined price basis of the synchronized drawings; the receiver adopts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_basis: Option<String>,
}

/// One multiplicative price-basis segment for
/// [`ChartEngine::rescale_drawing_prices`](crate::ChartEngine::rescale_drawing_prices): anchors
/// whose time lies in `[from_time, to_time)` (UTC seconds; `None` is unbounded) are multiplied by
/// `factor`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingPriceSegment {
    #[serde(default)]
    pub from_time: Option<f64>,
    #[serde(default)]
    pub to_time: Option<f64>,
    pub factor: f64,
}

impl DrawingPriceSegment {
    /// Smallest and largest accepted factors; adjustment ratios are far inside this range and the
    /// bound keeps every rescaled finite price finite.
    pub const MIN_FACTOR: f64 = 1e-6;
    pub const MAX_FACTOR: f64 = 1e6;

    pub fn contains(&self, time: f64) -> bool {
        self.from_time.is_none_or(|from| time >= from) && self.to_time.is_none_or(|to| time < to)
    }

    /// Validate a segment list: bounded length, finite bounds with `from < to`, factors in
    /// `[MIN_FACTOR, MAX_FACTOR]`, and no two segments overlapping.
    pub fn validate_all(segments: &[Self]) -> Result<(), ChartError> {
        let invalid = |message: String| ChartError::new(ErrorCode::InvalidData, message);
        if segments.len() > MAX_DRAWING_PRICE_SEGMENTS {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                format!("at most {MAX_DRAWING_PRICE_SEGMENTS} price segments are supported"),
            ));
        }
        for (index, segment) in segments.iter().enumerate() {
            let finite_bound = |bound: Option<f64>| bound.is_none_or(f64::is_finite);
            if !finite_bound(segment.from_time) || !finite_bound(segment.to_time) {
                return Err(invalid(format!(
                    "price segment {index} has a non-finite bound"
                )));
            }
            if let (Some(from), Some(to)) = (segment.from_time, segment.to_time) {
                if from >= to {
                    return Err(invalid(format!("price segment {index} is empty")));
                }
            }
            if !(Self::MIN_FACTOR..=Self::MAX_FACTOR).contains(&segment.factor) {
                return Err(invalid(format!(
                    "price segment {index} factor must be in [{}, {}]",
                    Self::MIN_FACTOR,
                    Self::MAX_FACTOR
                )));
            }
        }
        let mut ordered = segments.to_vec();
        ordered.sort_by(|a, b| {
            a.from_time
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&b.from_time.unwrap_or(f64::NEG_INFINITY))
        });
        for pair in ordered.windows(2) {
            let previous_end = pair[0].to_time.unwrap_or(f64::INFINITY);
            let next_start = pair[1].from_time.unwrap_or(f64::NEG_INFINITY);
            if next_start < previous_end {
                return Err(invalid("price segments overlap".to_string()));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingTemplate {
    pub name: String,
    pub kind: DrawingKind,
    pub options: serde_json::Value,
}

impl DrawingTemplate {
    pub fn validate(&self) -> bool {
        !self.name.is_empty()
            && self.name.len() <= MAX_DRAWING_NAME_BYTES
            && self.options.is_object()
            && serde_json::to_vec(&self.options)
                .map(|bytes| bytes.len() <= MAX_DRAWING_TEMPLATE_BYTES)
                .unwrap_or(false)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DrawingCommonSnapshot {
    pub id: u32,
    pub kind: DrawingKind,
    pub name: String,
    pub group_id: Option<String>,
    pub revision: u64,
    pub visible: bool,
    pub locked: bool,
    pub z_order: i32,
    pub pane_index: usize,
    pub price_scale: DrawingPriceScale,
    pub magnet: DrawingMagnetMode,
    pub interval_visibility: DrawingIntervalVisibility,
    pub stroke_start: DrawingLineCap,
    pub stroke_end: DrawingLineCap,
    pub extend_left: bool,
    pub extend_right: bool,
    pub fill_enabled: bool,
    pub labels: Vec<DrawingLabelOptions>,
    pub levels: Vec<DrawingLevel>,
}

/// Kind-specific option block projected from the authoritative live drawing.  Hosts can use this
/// typed view instead of switching over the legacy flat options object; the engine intentionally
/// does not store a second copy of these values.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrawingKindOptions {
    Rectangle {
        fill_color: Option<String>,
        preview_fill_color: Option<String>,
        border_visible: bool,
        show_labels: bool,
        axis_bands_visible: bool,
        label_color: Option<String>,
        label_text_color: Option<String>,
        snap_time_to_data: bool,
    },
    Text {
        box_color: Option<String>,
        box_border_color: Option<String>,
        box_border_width: f64,
    },
    Position {
        levels: Vec<DrawingLevel>,
    },
    Generic,
    // B8: lines — begin
    /// Every Lines-family tool (`tool_options.line`, resolved).
    Line {
        stats_position: crate::DrawingStatsPosition,
    },
    // B8: lines — end
    // B8: channels — begin
    /// Parallel channel, flat top/bottom, and disjoint channel (`tool_options.channel`,
    /// resolved against the tool's defaults).
    Channel {
        middle_line: bool,
        middle_color: Option<String>,
    },
    /// Regression trend (`tool_options.channel`, resolved against the tool's defaults).
    RegressionTrend {
        middle_line: bool,
        middle_color: Option<String>,
        upper_deviation: f64,
        lower_deviation: f64,
        use_upper_deviation: bool,
        use_lower_deviation: bool,
        source: crate::IndicatorInputSource,
        show_pearsons: bool,
    },
    // B8: channels — end
    // B8: fibonacci — begin
    /// Every Fibonacci-family tool (`tool_options.fibonacci`, resolved with the kind's label
    /// placement defaults).
    Fibonacci(crate::FibonacciToolOptions),
    // B8: fibonacci — end
    // B8: pitchforks_gann — begin
    /// Every pitchfork and the pitchfan: the level list (median offsets in half-handle widths).
    Pitchfork {
        levels: Vec<DrawingLevel>,
    },
    /// Every Gann tool: its level list and the resolved `tool_options.gann` block.
    Gann {
        levels: Vec<DrawingLevel>,
        time_levels: Vec<DrawingLevel>,
        angles: Vec<DrawingLevel>,
        arcs: Vec<DrawingLevel>,
        reverse: bool,
        show_angles: bool,
        show_stats: bool,
        scale_ratio: Option<f64>,
        size_bars: f64,
    },
    // B8: pitchforks_gann — end
    // B8: projection_annotations — begin
    /// Every Projection & Annotations tool (`tool_options.projection_annotation`, resolved).
    /// `pattern_bars` is the number of bars a bars pattern captured.
    ProjectionAnnotation {
        bars_mode: crate::BarsPatternMode,
        mirrored: bool,
        flipped: bool,
        pattern_bars: usize,
        icon: crate::DrawingIcon,
        icon_size: f64,
        always_show_text: bool,
    },
    // B8: projection_annotations — end
    // B8: patterns_elliott_cycles — begin
    /// XABCD, cypher, ABCD, and three drives (`tool_options.pattern`, resolved).
    Pattern {
        show_ratios: bool,
    },
    /// Every Elliott wave tool (`tool_options.pattern`, resolved).
    ElliottWave {
        degree: crate::ElliottWaveDegree,
        show_wave: bool,
    },
    // B8: patterns_elliott_cycles — end
    // B8: shapes — begin
    /// Every Shapes-family tool (`tool_options.shape`, resolved).
    Shape {
        closed: bool,
    },
    // B8: shapes — end
}

/// Family-specific typed option blocks (B8), one optional block per drawing family; `None` means
/// that family's defaults. Options JSON, templates, clipboard/sync payloads, and persistence carry
/// the blocks under `tool_options`. Patches deep-merge: absent keys keep their values and `null`
/// resets a block.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DrawingToolOptions {
    // B8: lines — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<crate::LineToolOptions>,
    // B8: lines — end
    // B8: channels — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<crate::ChannelToolOptions>,
    // B8: channels — end
    // B8: fibonacci — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fibonacci: Option<crate::FibonacciToolOptions>,
    // B8: fibonacci — end
    // B8: pitchforks_gann — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gann: Option<crate::GannToolOptions>,
    // B8: pitchforks_gann — end
    // B8: projection_annotations — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_annotation: Option<crate::ProjectionAnnotationToolOptions>,
    // B8: projection_annotations — end
    // B8: patterns_elliott_cycles — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<crate::PatternToolOptions>,
    // B8: patterns_elliott_cycles — end
    // B8: shapes — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<crate::ShapeToolOptions>,
    // B8: shapes — end
}

/// Upper bound on one serialized `tool_options` object, keeping patches and documents bounded.
pub const MAX_DRAWING_TOOL_OPTIONS_BYTES: usize = 16 * 1024;

impl DrawingToolOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Validation shared by patches and persistence: the serialized size bound, then each family
    /// block's own checks (for example bounded list lengths or finite numbers), appended as
    /// `&& ...` inside that family's block.
    pub fn validate(&self) -> bool {
        serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= MAX_DRAWING_TOOL_OPTIONS_BYTES)
        // B8: lines — begin
        // B8: lines — end
        // B8: channels — begin
        && self
            .channel
            .as_ref()
            .is_none_or(crate::ChannelToolOptions::validate)
        // B8: channels — end
        // B8: fibonacci — begin
        // B8: fibonacci — end
        // B8: pitchforks_gann — begin
        && self.gann.as_ref().is_none_or(crate::GannToolOptions::validate)
        // B8: pitchforks_gann — end
        // B8: projection_annotations — begin
            && self
                .projection_annotation
                .as_ref()
                .is_none_or(crate::ProjectionAnnotationToolOptions::validate)
        // B8: projection_annotations — end
        // B8: patterns_elliott_cycles — begin
        // B8: patterns_elliott_cycles — end
        // B8: shapes — begin
        // B8: shapes — end
    }

    /// These options as a named template keeps them: data a drawing captured (not style) stays
    /// with that drawing like its anchors, so applying a template restyles a drawing without
    /// replacing its data. Each family clears its captured fields inside its own block.
    pub(crate) fn template_style(&self) -> Self {
        let mut style = self.clone();
        // B8: lines — begin
        // B8: lines — end
        // B8: channels — begin
        // B8: channels — end
        // B8: fibonacci — begin
        // B8: fibonacci — end
        // B8: pitchforks_gann — begin
        // B8: pitchforks_gann — end
        // B8: projection_annotations — begin
        // A bars pattern's copied bars.
        if let Some(block) = style.projection_annotation.as_mut() {
            block.bars.clear();
        }
        // B8: projection_annotations — end
        // B8: patterns_elliott_cycles — begin
        // B8: patterns_elliott_cycles — end
        // B8: shapes — begin
        // B8: shapes — end
        style
    }

    /// Deep-merge a JSON patch object into a copy of these options. `None` for a malformed or
    /// oversized result, leaving the caller's options untouched.
    pub(crate) fn merged(&self, patch: &serde_json::Value) -> Option<Self> {
        fn merge(target: &mut serde_json::Value, patch: &serde_json::Value) {
            match (target, patch) {
                (serde_json::Value::Object(target), serde_json::Value::Object(patch)) => {
                    for (key, value) in patch {
                        if value.is_null() {
                            target.remove(key);
                        } else {
                            merge(
                                target.entry(key.clone()).or_insert(serde_json::Value::Null),
                                value,
                            );
                        }
                    }
                }
                (target, patch) => *target = patch.clone(),
            }
        }
        if !patch.is_object() {
            return None;
        }
        let mut value = serde_json::to_value(self).ok()?;
        merge(&mut value, patch);
        let merged: Self = serde_json::from_value(value).ok()?;
        merged.validate().then_some(merged)
    }

    /// The patch that makes these options equal `target` under [`Self::merged`]: `target`'s
    /// values plus `null` for every block or field these options hold and `target` omits. A
    /// template applies its `tool_options` through it, so it replaces the drawing's family
    /// options (defaults it leaves unset included) instead of merging into them.
    pub(crate) fn replacement_patch(&self, target: &serde_json::Value) -> serde_json::Value {
        fn diff(current: &serde_json::Value, target: &serde_json::Value) -> serde_json::Value {
            match (current, target) {
                (serde_json::Value::Object(current), serde_json::Value::Object(target)) => {
                    let mut patch = serde_json::Map::new();
                    for (key, value) in target {
                        let value = match current.get(key) {
                            Some(held) => diff(held, value),
                            None => value.clone(),
                        };
                        patch.insert(key.clone(), value);
                    }
                    for key in current.keys() {
                        if !target.contains_key(key) {
                            patch.insert(key.clone(), serde_json::Value::Null);
                        }
                    }
                    serde_json::Value::Object(patch)
                }
                _ => target.clone(),
            }
        }
        serde_json::to_value(self).map_or_else(|_| target.clone(), |current| diff(&current, target))
    }
}

/// A property descriptor without bounds or enum values (the common schema's and each family's
/// `tool_options.*` rows).
pub(crate) fn descriptor(
    name: impl Into<String>,
    property_type: DrawingPropertyType,
    default: serde_json::Value,
) -> DrawingPropertyDescriptor {
    DrawingPropertyDescriptor {
        name: name.into(),
        property_type,
        default,
        min: None,
        max: None,
        enum_values: Vec::new(),
    }
}

/// Return the complete generic property schema for one built-in drawing kind.  The schema is
/// data, so a host can build a property panel without a tool-specific switch statement.
pub fn drawing_property_schema(kind: DrawingKind) -> DrawingPropertySchema {
    let mut properties = vec![
        descriptor("name", DrawingPropertyType::String, serde_json::json!("")),
        descriptor(
            "visible",
            DrawingPropertyType::Boolean,
            serde_json::json!(true),
        ),
        descriptor(
            "locked",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ),
        descriptor(
            "group_id",
            DrawingPropertyType::String,
            serde_json::json!(""),
        ),
        descriptor(
            "z_order",
            DrawingPropertyType::Integer,
            serde_json::json!(0),
        ),
        descriptor(
            "interval_visibility",
            DrawingPropertyType::IntervalSet,
            serde_json::json!({}),
        ),
        descriptor(
            "color",
            DrawingPropertyType::Color,
            serde_json::json!("#2962ff"),
        ),
        descriptor("width", DrawingPropertyType::Number, serde_json::json!(2.0)),
        descriptor(
            "style",
            DrawingPropertyType::Enum,
            serde_json::json!("solid"),
        ),
        descriptor(
            "stroke_start",
            DrawingPropertyType::Enum,
            serde_json::json!("none"),
        ),
        descriptor(
            "stroke_end",
            DrawingPropertyType::Enum,
            serde_json::json!("none"),
        ),
        descriptor(
            "extend_left",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ),
        descriptor(
            "extend_right",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ),
        descriptor(
            "fill_enabled",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ),
        descriptor(
            "fill_color",
            DrawingPropertyType::Color,
            serde_json::json!(""),
        ),
        descriptor("text", DrawingPropertyType::String, serde_json::json!("")),
        descriptor(
            "text_color",
            DrawingPropertyType::Color,
            serde_json::json!(""),
        ),
        descriptor(
            "text_size",
            DrawingPropertyType::Number,
            serde_json::Value::Null,
        ),
        descriptor(
            "text_weight",
            DrawingPropertyType::Integer,
            serde_json::json!(400),
        ),
        descriptor(
            "text_italic",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ),
        descriptor(
            "text_h_align",
            DrawingPropertyType::Enum,
            serde_json::json!("center"),
        ),
        descriptor(
            "text_v_align",
            DrawingPropertyType::Enum,
            serde_json::json!("middle"),
        ),
        descriptor("labels", DrawingPropertyType::Levels, serde_json::json!([])),
        descriptor("levels", DrawingPropertyType::Levels, serde_json::json!([])),
        descriptor(
            "magnet",
            DrawingPropertyType::Enum,
            serde_json::json!("off"),
        ),
        descriptor("points", DrawingPropertyType::Points, serde_json::json!([])),
        descriptor(
            "price_scale_id",
            DrawingPropertyType::Enum,
            serde_json::json!("right"),
        ),
    ];
    for property in &mut properties {
        if property.name == "style" {
            property.enum_values = ["solid", "dotted", "dashed", "large_dashed", "sparse_dotted"]
                .into_iter()
                .map(str::to_string)
                .collect();
        } else if matches!(property.name.as_str(), "stroke_start" | "stroke_end") {
            property.enum_values = ["none", "arrow", "circle"]
                .into_iter()
                .map(str::to_string)
                .collect();
        } else if property.name == "magnet" {
            property.enum_values = ["off", "weak", "strong"]
                .into_iter()
                .map(str::to_string)
                .collect();
        } else if property.name == "price_scale_id" {
            property.enum_values = ["left", "right", "overlay"]
                .into_iter()
                .map(str::to_string)
                .collect();
        }
    }
    if let Some(family) = kind.spec().family {
        // Family kinds report their own resolved defaults (a ray's `extend_right`, an info
        // line's stats) before appending their `tool_options.*` descriptors.
        let template = crate::Drawing::new(0, kind, 0, Vec::new());
        crate::drawings::kinds::apply_template_defaults(&template, &mut properties);
        (family.extend_schema)(&template, &mut properties);
    }
    DrawingPropertySchema {
        revision: DRAWING_CONTRACT_REVISION,
        kind,
        properties,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_tables_resolve_prices_and_labels() {
        let levels = drawing_levels_from_ratios(&FIBONACCI_RATIOS, "#787b86");
        assert_eq!(levels.len(), FIBONACCI_RATIOS.len());
        assert!(levels.iter().all(|level| level.validate() && level.visible));
        let golden = &levels[4];
        assert!((golden.price_between(100.0, 200.0) - 161.8).abs() < 1e-9);
        assert!(
            (levels[7].price_between(200.0, 100.0) - 38.2).abs() < 1e-9,
            "extension"
        );
        assert_eq!(golden.label(true, Some("161.80")), "0.618 (161.80)");
        assert_eq!(levels[6].label(true, None), "1");
        assert_eq!(golden.label(false, Some("161.80")), "161.80");
        assert_eq!(golden.label(false, None), "");
        assert!(FIBONACCI_TIME_ZONES
            .windows(3)
            .skip(1)
            .all(|w| w[2] == w[0] + w[1]));
    }
}
