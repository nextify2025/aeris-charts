//! Typed drawing customization contracts shared by native and browser hosts.
//!
//! The engine keeps the live [`Drawing`](crate::Drawing) representation compact for the hot
//! render and hit-test paths.  This module is the stable, typed view used by property panels,
//! templates, persistence adapters, and cross-cell synchronization.  Values are deliberately
//! bounded and deterministic so a host cannot turn a drawing patch into unbounded work.

use crate::{ChartError, DrawingAnchor, DrawingKind, DrawingPriceScale, ErrorCode};

pub const DRAWING_CONTRACT_REVISION: u32 = 1;
pub const MAX_DRAWING_NAME_BYTES: usize = 256;
pub const MAX_DRAWING_GROUP_BYTES: usize = 128;
pub const MAX_DRAWING_LABELS: usize = 32;
pub const MAX_DRAWING_LEVELS: usize = 64;
pub const MAX_DRAWING_TEMPLATE_BYTES: usize = 64 * 1024;
pub const MAX_DRAWING_TEMPLATES: usize = 128;
pub const MAX_DRAWING_OBJECTS: usize = 10_000;
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
}

fn descriptor(
    name: &str,
    property_type: DrawingPropertyType,
    default: serde_json::Value,
) -> DrawingPropertyDescriptor {
    DrawingPropertyDescriptor {
        name: name.to_string(),
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
    DrawingPropertySchema {
        revision: DRAWING_CONTRACT_REVISION,
        kind,
        properties,
    }
}
