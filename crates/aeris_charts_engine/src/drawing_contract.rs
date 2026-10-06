//! Typed drawing customization contracts shared by native and browser hosts.
//!
//! The engine keeps the live [`Drawing`](crate::Drawing) representation compact for the hot
//! render and hit-test paths.  This module is the stable, typed view used by property panels,
//! templates, persistence adapters, and cross-cell synchronization.  Values are deliberately
//! bounded and deterministic so a host cannot turn a drawing patch into unbounded work.

use crate::{ChartError, DrawingAnchor, DrawingKind, DrawingPriceScale, ErrorCode};

pub const DRAWING_CONTRACT_REVISION: u32 = 1;
pub const MAX_DRAWING_NAME_BYTES: usize = 256;
/// Byte bound of one drawing's `text`, on every path that sets it: options and patches reject a
/// longer text, and the inline editor's live text clamps to it. Persistence accepts exactly this
/// much per drawing (and bounds the document's total separately).
pub const MAX_DRAWING_TEXT_BYTES: usize = 65_536;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars_pattern: Option<Vec<crate::drawings::BarsPatternBar>>,
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
    AnchoredText {
        screen_x: f64,
        screen_y: f64,
        box_color: Option<String>,
        box_border_color: Option<String>,
        box_border_width: f64,
    },
    IconStamp {
        icon_name: Option<String>,
        icon_size: f64,
    },
    BarsPattern {
        mirror_x: bool,
        mirror_y: bool,
        mode: String,
        bar_count: usize,
    },
    Position {
        levels: Vec<DrawingLevel>,
        account_size: f64,
        risk_percent: f64,
    },
    Levels {
        levels: Vec<DrawingLevel>,
        reverse: bool,
        log_scale: bool,
        show_prices: bool,
        show_values: bool,
        show_percents: bool,
        label_align: String,
    },
    GannSquare {
        levels: Vec<DrawingLevel>,
        fans: Vec<DrawingLevel>,
        arcs: Vec<DrawingLevel>,
        reverse: bool,
        show_prices: bool,
        show_values: bool,
        show_percents: bool,
        label_align: String,
    },
    RegressionTrend {
        source_id: Option<u32>,
        deviations: f64,
    },
    Elliott {
        wave_degree: String,
    },
    Generic,
    // B8: lines — begin
    /// The own-line line tools (horizontal segment, vertical ray, vertical segment, price line):
    /// `tool_options.line`, resolved.
    Line {
        stats_position: crate::DrawingStatsPosition,
    },
    // B8: lines — end
    // B8: channels — begin
    /// The own-line price channel (`tool_options.channel`, resolved against the tool's
    /// defaults). Upstream's channels keep [`Self::Generic`] and its regression trend keeps
    /// [`Self::RegressionTrend`]; their `channel` block reaches hosts through `options_json`.
    Channel {
        middle_line: bool,
        middle_color: Option<String>,
    },
    // B8: channels — end
    // B8: projection_annotations — begin
    /// The own-line annotation tools (simple tag, simple annotation) and the measuring ranges
    /// (price range, date range, date and price range): `tool_options.projection_annotation`,
    /// resolved. `pattern_bars` is the number of bars the block's `bars` holds.
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
}

/// The fork's typed option blocks (B8), one optional block per fork drawing family; `None` means
/// that family's defaults. Options JSON, templates, clipboard/sync payloads, and persistence carry
/// the blocks under `tool_options`. Patches deep-merge: absent keys keep their values and `null`
/// resets a block.
///
/// The upstream catalog's flat [`Drawing`](crate::Drawing) fields are canonical. A block key that
/// overlaps one of them (a Fibonacci tool's `fibonacci.reverse`, a Gann square's `gann.angles`,
/// an Elliott wave's `pattern.degree`, a bars pattern's `projection_annotation.bars`, an icon
/// stamp's `projection_annotation.icon`) is an input alias of that field: patches, templates,
/// paste, and persistence move it onto the field through [`take_legacy_flat_options`], and the
/// flat key wins when both are given. Every other key (a regression trend's per-side `channel`
/// deviations included) is stored and persisted, and read by the family kinds (the own-line tools
/// and the ranges); of the upstream-rendered kinds, the six line tools read `line` (its presence
/// layers the fork's stats box, trend-angle decorations and arrowheads on their upstream arms),
/// the parallel, flat and disjoint channels read `channel`'s middle line, the regression trend
/// reads all of `channel` (middle line as its dashed centre, per-side deviations and switches,
/// source, Pearson's R), the Fibonacci tools read `fibonacci` (trend line, fan grid, full circles,
/// vertical label placement, the golden spiral's turn; a stored block also selects the ring
/// tools' precise rings), the harmonic patterns read `pattern.show_ratios` and the Elliott waves
/// `pattern.show_wave`, and the others do not read their keys yet. Each block's defaults are
/// upstream's look, and documents and payloads the fork wrote carry the fork's unstored defaults
/// explicitly (`drawings::kinds::legacy_fork_tool_options`).
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fibonacci: Option<crate::FibonacciToolOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gann: Option<crate::GannToolOptions>,
    // B8: projection_annotations — begin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_annotation: Option<crate::ProjectionAnnotationToolOptions>,
    // B8: projection_annotations — end
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<crate::PatternToolOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<crate::ShapeToolOptions>,
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
        && self.gann.as_ref().is_none_or(crate::GannToolOptions::validate)
        // B8: projection_annotations — begin
            && self
                .projection_annotation
                .as_ref()
                .is_none_or(crate::ProjectionAnnotationToolOptions::validate)
        // B8: projection_annotations — end
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
        // B8: projection_annotations — begin
        // Copied bars in the fork's block (a bars pattern's legacy `bars` key).
        if let Some(block) = style.projection_annotation.as_mut() {
            block.bars.clear();
        }
        // B8: projection_annotations — end
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

/// Upstream flat option values read from fork `tool_options` keys that overlap them (see
/// [`take_legacy_flat_options`]). A field is `Some` only when its fork key supplied it (or, for a
/// legacy document, the fork block's default did); the caller applies it where the input carried
/// no flat key of its own, through that flat key's validation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LegacyFlatOptions {
    pub(crate) level_reverse: Option<bool>,
    pub(crate) level_log_scale: Option<bool>,
    pub(crate) level_show_prices: Option<bool>,
    pub(crate) level_show_values: Option<bool>,
    pub(crate) level_show_percents: Option<bool>,
    pub(crate) level_label_align: Option<String>,
    pub(crate) gann_fans: Option<Vec<DrawingLevel>>,
    pub(crate) gann_arcs: Option<Vec<DrawingLevel>>,
    pub(crate) wave_degree: Option<String>,
    pub(crate) bars_pattern: Option<Vec<crate::drawings::BarsPatternBar>>,
    pub(crate) bars_pattern_mirror_x: Option<bool>,
    pub(crate) bars_pattern_mirror_y: Option<bool>,
    pub(crate) bars_pattern_mode: Option<String>,
    pub(crate) icon_name: Option<String>,
    pub(crate) icon_size: Option<f64>,
    pub(crate) regression_deviations: Option<f64>,
}

/// One fork option block being read for [`take_legacy_flat_options`].
struct LegacyBlock {
    /// The block's stored keys. Taken keys leave it; the rest goes back into `tool_options`.
    stored: serde_json::Map<String, serde_json::Value>,
    /// The fork block's default keys, read under the stored ones for a legacy document (the fork
    /// deserialized every block with `#[serde(default)]`); empty otherwise.
    defaults: serde_json::Map<String, serde_json::Value>,
}

impl LegacyBlock {
    /// The value of `key`, removed from the stored block. A stored value `read` rejects stays,
    /// so the block's own deserialization reports it instead of the key vanishing silently.
    fn take<T>(&mut self, key: &str, read: impl Fn(&serde_json::Value) -> Option<T>) -> Option<T> {
        if let Some(value) = self.stored.get(key) {
            let value = read(value)?;
            self.stored.remove(key);
            return Some(value);
        }
        self.defaults.get(key).and_then(read)
    }
}

/// A fork Gann level list (`gann.angles` or `gann.arcs`) as an upstream Gann family: entries the
/// upstream family rejects are dropped and the list keeps at most [`MAX_DRAWING_LEVELS`].
fn legacy_gann_family(value: &serde_json::Value, fan: bool) -> Option<Vec<DrawingLevel>> {
    let levels: Vec<DrawingLevel> = serde_json::from_value(value.clone()).ok()?;
    Some(
        levels
            .into_iter()
            .filter(|level| crate::drawings::valid_gann_family(std::slice::from_ref(level), fan))
            .take(MAX_DRAWING_LEVELS)
            .collect(),
    )
}

/// Fork bars (`projection_annotation.bars`, `[open, high, low, close]` oldest first) as an
/// upstream snapshot: each bar keeps its list position as its offset, invalid bars are dropped,
/// and offsets stay below [`MAX_BARS_PATTERN_BARS`](crate::drawings::MAX_BARS_PATTERN_BARS).
pub(crate) fn legacy_bars_pattern(bars: &[[f64; 4]]) -> Vec<crate::drawings::BarsPatternBar> {
    bars.iter()
        .take(crate::drawings::MAX_BARS_PATTERN_BARS)
        .enumerate()
        .filter_map(|(offset, &[open, high, low, close])| {
            let bar = crate::drawings::BarsPatternBar {
                offset: u16::try_from(offset).ok()?,
                open,
                high,
                low,
                close,
            };
            bar.valid().then_some(bar)
        })
        .collect()
}

/// Move the fork `tool_options` keys of `kind` that overlap an upstream flat field out of
/// `tool_options` and return them as that field's normalized value. It works by key presence: a
/// patch that sends part of a block maps exactly the keys it sends. A block left empty is removed;
/// every other key stays stored: the own-line families read theirs, and so do the upstream kinds
/// that layer a presentation on their arms (the line tools' `line`, the channels' and the
/// regression trend's `channel`, the Fibonacci tools' `fibonacci`, the harmonic patterns'
/// `pattern.show_ratios` and the Elliott waves' `pattern.show_wave`), while the other
/// upstream-rendered kinds do not read them yet.
///
/// With `absent_block_is_default` (a document the fork wrote, which omitted values equal to its
/// defaults) the fork block's defaults stand in for absent keys, including a block that is absent
/// altogether: [`FibonacciToolOptions`](crate::FibonacciToolOptions),
/// [`GannToolOptions`](crate::GannToolOptions), [`PatternToolOptions`](crate::PatternToolOptions),
/// [`ProjectionAnnotationToolOptions`](crate::ProjectionAnnotationToolOptions), or
/// [`ChannelToolOptions`](crate::ChannelToolOptions).
///
/// The mapping, by kind:
///
/// - Fibonacci tools, block `fibonacci`: `reverse` to `level_reverse` where the fork read it with
///   upstream's meaning: as is on the extension, channel, and time zones, inverted on the
///   retracement and the speed fan (the fork's level 0 sat on their second anchor, upstream's on the
///   first). The spiral's `reverse` (the golden spiral's counterclockwise turn) has no flat
///   counterpart and stays stored, and the tools the fork never reversed have nothing to map, so
///   their `reverse` stays stored but inert.
///   `log_scale` to `level_log_scale` (retracement, extension, channel); `show_prices` to
///   `level_show_prices`;
///   `show_levels` and `levels_as_percent` to `level_show_values` and `level_show_percents`;
///   `label_h_align` to `level_label_align`, with `left` and `right` swapped on the time zones
///   and trend time (the fork named the side of the line its label sat on; upstream names the
///   edge of the text anchored at the line, so its `left` puts the label right of the line).
/// - Gann box, squares, and fan, block `gann`: `reverse` to `level_reverse` (a fork document's
///   fan drops it: the fork's fan never read it); on the squares, `angles` and `arcs` to
///   `gann_fans` and `gann_arcs`.
/// - Elliott waves, block `pattern`: `degree` to `wave_degree`.
/// - Bars pattern, block `projection_annotation`: `bars_mode` to `bars_pattern_mode`, `mirrored`
///   and `flipped` to `bars_pattern_mirror_x` and `bars_pattern_mirror_y`, `bars` to
///   `bars_pattern`.
/// - Icon stamp, block `projection_annotation`: `icon` and `icon_size` to `icon_name` and
///   `icon_size`.
/// - Regression trend, block `channel`: nothing is taken. The two deviation sides and their
///   switches are per-side overrides of `regression_deviations` and stay in the block; for a
///   fork document they also fold into `regression_deviations` (the wider enabled side), and a
///   symmetric band with both sides on leaves no key behind (it is that flat value alone).
pub(crate) fn take_legacy_flat_options(
    kind: DrawingKind,
    tool_options: &mut serde_json::Value,
    absent_block_is_default: bool,
) -> LegacyFlatOptions {
    let mut legacy = LegacyFlatOptions::default();
    let fibonacci = kind.is_fibonacci();
    let gann = matches!(
        kind,
        DrawingKind::GannBox
            | DrawingKind::GannSquare
            | DrawingKind::GannSquareFixed
            | DrawingKind::GannFan
    );
    let name = if fibonacci {
        "fibonacci"
    } else if gann {
        "gann"
    } else if kind.is_elliott() {
        "pattern"
    } else if matches!(kind, DrawingKind::BarsPattern | DrawingKind::IconStamp) {
        "projection_annotation"
    } else if kind == DrawingKind::RegressionTrend {
        "channel"
    } else {
        return legacy;
    };
    let Some(options) = tool_options.as_object_mut() else {
        return legacy;
    };
    let stored = match options.remove(name) {
        Some(serde_json::Value::Object(stored)) => stored,
        // A `null` reset or a malformed block stays for the caller's own handling.
        Some(other) => {
            options.insert(name.to_string(), other);
            return legacy;
        }
        None if absent_block_is_default => serde_json::Map::new(),
        None => return legacy,
    };
    let defaults = if absent_block_is_default {
        let defaults = match name {
            "fibonacci" => serde_json::to_value(crate::FibonacciToolOptions::default()),
            "gann" => serde_json::to_value(crate::GannToolOptions::default()),
            "pattern" => serde_json::to_value(crate::PatternToolOptions::default()),
            // Written out: the block serializes nothing at its defaults.
            "projection_annotation" => {
                let defaults = crate::ProjectionAnnotationToolOptions::default();
                Ok(serde_json::json!({
                    "bars_mode": defaults.bars_mode,
                    "mirrored": defaults.mirrored,
                    "flipped": defaults.flipped,
                    "icon": defaults.icon,
                    "icon_size": defaults.icon_size,
                }))
            }
            _ => serde_json::to_value(crate::ChannelToolOptions::default()),
        };
        match defaults {
            Ok(serde_json::Value::Object(defaults)) => defaults,
            _ => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };
    let mut block = LegacyBlock { stored, defaults };
    let boolean = serde_json::Value::as_bool;
    let number = serde_json::Value::as_f64;
    let text = |value: &serde_json::Value| value.as_str().map(str::to_string);
    if fibonacci {
        legacy.level_reverse = match kind {
            // The fork's level 0 sat on the second anchor, upstream's sits on the first.
            DrawingKind::FibonacciRetracement | DrawingKind::FibonacciSpeedFan => {
                block.take("reverse", boolean).map(|reverse| !reverse)
            }
            DrawingKind::FibonacciExtension
            | DrawingKind::FibonacciChannel
            | DrawingKind::FibonacciTimeZones => block.take("reverse", boolean),
            _ => None,
        };
        if kind.supports_log_levels() {
            legacy.level_log_scale = block.take("log_scale", boolean);
        }
        legacy.level_show_prices = block.take("show_prices", boolean);
        let show_levels = block.take("show_levels", boolean);
        let as_percent = block.take("levels_as_percent", boolean);
        if show_levels.is_some() || as_percent.is_some() {
            let show_levels = show_levels.unwrap_or(true);
            let as_percent = as_percent.unwrap_or(false);
            legacy.level_show_values = Some(show_levels && !as_percent);
            legacy.level_show_percents = Some(show_levels && as_percent);
        }
        let time = matches!(
            kind,
            DrawingKind::FibonacciTimeZones | DrawingKind::FibonacciTrendTime
        );
        legacy.level_label_align = block
            .take("label_h_align", text)
            // The fork's unset alignment is the tool's own: time levels label their right.
            .or_else(|| {
                absent_block_is_default.then(|| (if time { "right" } else { "left" }).into())
            })
            // The fork put a time level's label on the named side of its line; upstream's `left`
            // and `right` name the edge of the text anchored at the line, so the label sits on
            // the other side (`left` puts it right of the line).
            .map(|align| match align.as_str() {
                "left" if time => "right".to_string(),
                "right" if time => "left".to_string(),
                _ => align,
            });
    } else if gann {
        // The fork's fan never read `reverse` (only a block's generic serialization carried it),
        // so a document's is dropped; a patch keeps the documented alias.
        let reverse = block.take("reverse", boolean);
        legacy.level_reverse =
            reverse.filter(|_| !(absent_block_is_default && kind == DrawingKind::GannFan));
        if matches!(kind, DrawingKind::GannSquare | DrawingKind::GannSquareFixed) {
            legacy.gann_fans = block.take("angles", |value| legacy_gann_family(value, true));
            legacy.gann_arcs = block.take("arcs", |value| legacy_gann_family(value, false));
        }
    } else if kind.is_elliott() {
        legacy.wave_degree = block.take("degree", text);
    } else if kind == DrawingKind::BarsPattern {
        legacy.bars_pattern_mode = block.take("bars_mode", text).map(|mode| {
            if mode == "hl_bars" {
                "bars".to_string()
            } else {
                mode
            }
        });
        legacy.bars_pattern_mirror_x = block.take("mirrored", boolean);
        legacy.bars_pattern_mirror_y = block.take("flipped", boolean);
        legacy.bars_pattern = block.take("bars", |value| {
            serde_json::from_value::<Vec<[f64; 4]>>(value.clone())
                .ok()
                .map(|bars| legacy_bars_pattern(&bars))
        });
    } else if kind == DrawingKind::IconStamp {
        legacy.icon_name = block.take("icon", text);
        legacy.icon_size = block
            .take("icon_size", number)
            .map(|size| size.clamp(8.0, 96.0));
    } else {
        // Regression trend: the fork's deviation sides and their switches are per-side overrides
        // of `regression_deviations` and stay in the block. A fork document's band also folds
        // into that flat field (the wider enabled side), so readers of upstream's contract see
        // the nearest symmetric band; a symmetric band with both sides on is nothing but that,
        // and leaves no key behind, while any other band keeps its sides, an omitted side
        // written as the fork's default, so it no longer depends on the fold.
        if absent_block_is_default {
            let read = |key: &str, read: fn(&serde_json::Value) -> Option<f64>, default: f64| {
                block.stored.get(key).and_then(read).unwrap_or(default)
            };
            let switch = |key: &str| block.stored.get(key).and_then(boolean).unwrap_or(true);
            let (upper, lower) = (
                read("upper_deviation", number, 2.0),
                read("lower_deviation", number, -2.0),
            );
            let (use_upper, use_lower) =
                (switch("use_upper_deviation"), switch("use_lower_deviation"));
            let side = |enabled: bool, value: f64| if enabled { value.abs() } else { 0.0 };
            legacy.regression_deviations = Some(
                side(use_upper, upper)
                    .max(side(use_lower, lower))
                    .clamp(0.0, 10.0),
            );
            if use_upper && use_lower && upper == -lower && upper.abs() <= 10.0 {
                for key in [
                    "upper_deviation",
                    "lower_deviation",
                    "use_upper_deviation",
                    "use_lower_deviation",
                ] {
                    block.stored.remove(key);
                }
            } else {
                block
                    .stored
                    .entry("upper_deviation")
                    .or_insert(serde_json::json!(upper));
                block
                    .stored
                    .entry("lower_deviation")
                    .or_insert(serde_json::json!(lower));
            }
        }
    }
    if !block.stored.is_empty() {
        options.insert(name.to_string(), serde_json::Value::Object(block.stored));
    }
    legacy
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
    let defaults = crate::drawings::Drawing::new(0, kind, 0, Vec::new());
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
        descriptor(
            "width",
            DrawingPropertyType::Number,
            serde_json::json!(defaults.width),
        ),
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
            serde_json::to_value(defaults.stroke_end).unwrap_or_default(),
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
            serde_json::json!(defaults.fill_enabled),
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
        descriptor(
            "labels",
            DrawingPropertyType::Levels,
            serde_json::to_value(defaults.labels).unwrap_or_default(),
        ),
        descriptor(
            "levels",
            DrawingPropertyType::Levels,
            serde_json::to_value(defaults.levels).unwrap_or_default(),
        ),
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
    if matches!(kind, DrawingKind::LongPosition | DrawingKind::ShortPosition) {
        for (name, default, min, max) in [
            ("position_account_size", 1000.0, f64::MIN_POSITIVE, 1e15),
            ("position_risk_percent", 25.0, 0.0, 100.0),
        ] {
            let mut property = descriptor(
                name,
                DrawingPropertyType::Number,
                serde_json::json!(default),
            );
            property.min = Some(min);
            property.max = Some(max);
            properties.push(property);
        }
    }
    if kind == DrawingKind::RegressionTrend {
        properties.push(descriptor(
            "regression_source_id",
            DrawingPropertyType::Integer,
            serde_json::Value::Null,
        ));
        let mut deviations = descriptor(
            "regression_deviations",
            DrawingPropertyType::Number,
            serde_json::json!(2.0),
        );
        deviations.min = Some(0.0);
        deviations.max = Some(10.0);
        properties.push(deviations);
    }
    if kind.is_elliott() {
        let mut degree = descriptor(
            "wave_degree",
            DrawingPropertyType::Enum,
            serde_json::json!("minor"),
        );
        degree.enum_values = [
            "subminuette",
            "minuette",
            "minute",
            "minor",
            "intermediate",
            "primary",
            "cycle",
            "supercycle",
            "grand_supercycle",
            "submillennium",
            "millennium",
            "supermillennium",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        properties.push(degree);
    }
    if kind == DrawingKind::AnchoredText {
        for name in ["screen_x", "screen_y"] {
            let mut position =
                descriptor(name, DrawingPropertyType::Number, serde_json::json!(0.5));
            position.min = Some(0.0);
            position.max = Some(1.0);
            properties.push(position);
        }
    }
    if kind == DrawingKind::IconStamp {
        properties.push(descriptor(
            "icon_name",
            DrawingPropertyType::String,
            serde_json::json!(""),
        ));
        let mut size = descriptor(
            "icon_size",
            DrawingPropertyType::Number,
            serde_json::json!(24.0),
        );
        size.min = Some(8.0);
        size.max = Some(96.0);
        properties.push(size);
    }
    if kind == DrawingKind::BarsPattern {
        properties.push(descriptor(
            "bars_pattern_mirror_x",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ));
        properties.push(descriptor(
            "bars_pattern_mirror_y",
            DrawingPropertyType::Boolean,
            serde_json::json!(false),
        ));
        let mut mode = descriptor(
            "bars_pattern_mode",
            DrawingPropertyType::Enum,
            serde_json::json!("bars"),
        );
        mode.enum_values = [
            "bars",
            "oc_bars",
            "line_open",
            "line_high",
            "line_low",
            "line_close",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        properties.push(mode);
    }
    if kind.has_levels() {
        for (name, default) in [
            ("level_reverse", defaults.level_reverse),
            ("level_show_prices", defaults.level_show_prices),
            ("level_show_values", defaults.level_show_values),
            ("level_show_percents", defaults.level_show_percents),
        ] {
            properties.push(descriptor(
                name,
                DrawingPropertyType::Boolean,
                serde_json::json!(default),
            ));
        }
        if kind.supports_log_levels() {
            properties.push(descriptor(
                "level_log_scale",
                DrawingPropertyType::Boolean,
                serde_json::json!(false),
            ));
        }
        let mut align = descriptor(
            "level_label_align",
            DrawingPropertyType::Enum,
            serde_json::json!(defaults.level_label_align),
        );
        align.enum_values = ["left", "center", "right"]
            .into_iter()
            .map(str::to_string)
            .collect();
        properties.push(align);
    }
    if matches!(kind, DrawingKind::GannSquare | DrawingKind::GannSquareFixed) {
        properties.push(descriptor(
            "gann_fans",
            DrawingPropertyType::Levels,
            serde_json::to_value(defaults.gann_fans).unwrap_or_default(),
        ));
        properties.push(descriptor(
            "gann_arcs",
            DrawingPropertyType::Levels,
            serde_json::to_value(defaults.gann_arcs).unwrap_or_default(),
        ));
    }
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
    let template = crate::Drawing::new(0, kind, 0, Vec::new());
    if let Some(family) = kind.spec().family {
        // Family kinds report their own resolved defaults (a vertical ray's `extend_right`)
        // before appending their `tool_options.*` descriptors.
        crate::drawings::kinds::apply_template_defaults(&template, &mut properties);
        (family.extend_schema)(&template, &mut properties);
    } else {
        // Upstream kinds list the stored options their layered parts read (an upstream line
        // tool's `tool_options.line.stats_position`).
        crate::drawings::kinds::extend_upstream_schema(kind, &template, &mut properties);
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

    #[test]
    fn legacy_tool_option_keys_move_onto_flat_fields_by_key_presence() {
        // A partial patch maps exactly the keys it sends; the rest of the block stays, and a key
        // whose value is malformed stays for the block's own validation.
        let mut options = serde_json::json!({
            "fibonacci": {"reverse": true, "trend_line": false, "show_prices": "yes"},
            "shape": {"closed": true}
        });
        let legacy =
            take_legacy_flat_options(DrawingKind::FibonacciRetracement, &mut options, false);
        // The fork's reversed retracement put level 0 on the first anchor: upstream's unreversed
        // placement.
        assert_eq!(
            legacy,
            LegacyFlatOptions {
                level_reverse: Some(false),
                ..LegacyFlatOptions::default()
            }
        );
        assert_eq!(
            options,
            serde_json::json!({
                "fibonacci": {"trend_line": false, "show_prices": "yes"},
                "shape": {"closed": true}
            })
        );
        // One side of the value/percent pair takes the other's fork default; an emptied block
        // is removed.
        let mut options = serde_json::json!({"fibonacci": {"levels_as_percent": true}});
        let legacy = take_legacy_flat_options(DrawingKind::FibonacciChannel, &mut options, false);
        assert_eq!(legacy.level_show_values, Some(false));
        assert_eq!(legacy.level_show_percents, Some(true));
        assert_eq!(legacy.level_label_align, None);
        assert_eq!(options, serde_json::json!({}));
        // The channel reversed the way upstream does; the spiral's counterclockwise `reverse`
        // has no flat counterpart and stays stored.
        let mut options = serde_json::json!({"fibonacci": {"reverse": true}});
        let legacy = take_legacy_flat_options(DrawingKind::FibonacciChannel, &mut options, false);
        assert_eq!(legacy.level_reverse, Some(true));
        let mut options = serde_json::json!({"fibonacci": {"reverse": true}});
        let legacy = take_legacy_flat_options(DrawingKind::FibonacciSpiral, &mut options, false);
        assert_eq!(legacy, LegacyFlatOptions::default());
        assert_eq!(options, serde_json::json!({"fibonacci": {"reverse": true}}));
        // Another kind's block is not this kind's to map.
        let mut options = serde_json::json!({"fibonacci": {"reverse": true}});
        let legacy = take_legacy_flat_options(DrawingKind::TrendLine, &mut options, true);
        assert_eq!(legacy, LegacyFlatOptions::default());
        assert_eq!(options, serde_json::json!({"fibonacci": {"reverse": true}}));
    }

    #[test]
    fn legacy_tool_option_keys_keep_the_meaning_the_fork_gave_them() {
        // The fork named the side of a time level its label sat on; upstream names the edge of
        // the text anchored at the line (`left` puts the label right of it). Price levels mean
        // the same in both.
        for (kind, sent, mapped) in [
            (DrawingKind::FibonacciTimeZones, "right", "left"),
            (DrawingKind::FibonacciTrendTime, "left", "right"),
            (DrawingKind::FibonacciTimeZones, "center", "center"),
            (DrawingKind::FibonacciRetracement, "right", "right"),
        ] {
            let mut options = serde_json::json!({"fibonacci": {"label_h_align": sent}});
            let legacy = take_legacy_flat_options(kind, &mut options, false);
            assert_eq!(
                legacy.level_label_align.as_deref(),
                Some(mapped),
                "{kind:?}"
            );
        }
        // The fork's fan never read `reverse`: a document's is dropped, a patch's maps.
        let mut options = serde_json::json!({"gann": {"reverse": true, "scale_ratio": 0.5}});
        let legacy = take_legacy_flat_options(DrawingKind::GannFan, &mut options, true);
        assert_eq!(legacy.level_reverse, None);
        assert_eq!(options, serde_json::json!({"gann": {"scale_ratio": 0.5}}));
        let mut options = serde_json::json!({"gann": {"reverse": true}});
        let legacy = take_legacy_flat_options(DrawingKind::GannFan, &mut options, false);
        assert_eq!(legacy.level_reverse, Some(true));
        let mut options = serde_json::json!({"gann": {"reverse": true}});
        let legacy = take_legacy_flat_options(DrawingKind::GannBox, &mut options, true);
        assert_eq!(legacy.level_reverse, Some(true));
        // The annotation block serializes nothing at its defaults, yet a fork document's absent
        // keys still read as the fork's defaults.
        assert_eq!(
            serde_json::to_value(crate::ProjectionAnnotationToolOptions::default()).unwrap(),
            serde_json::json!({})
        );
        let mut options = serde_json::json!({});
        let legacy = take_legacy_flat_options(DrawingKind::IconStamp, &mut options, true);
        assert_eq!(legacy.icon_name.as_deref(), Some("star"));
        assert_eq!(legacy.icon_size, Some(24.0));
        let legacy = take_legacy_flat_options(DrawingKind::BarsPattern, &mut options, true);
        assert_eq!(legacy.bars_pattern_mode.as_deref(), Some("bars"));
        assert_eq!(legacy.bars_pattern_mirror_x, Some(false));
        assert_eq!(options, serde_json::json!({}));
    }

    #[test]
    fn legacy_tool_option_blocks_fold_into_upstream_values() {
        // The regression's sides and switches are per-side overrides: a patch keeps them all and
        // leaves `regression_deviations` alone.
        let sides = serde_json::json!({"channel": {
            "upper_deviation": 3.0,
            "lower_deviation": -4.5,
            "use_lower_deviation": false,
            "middle_line": true
        }});
        let mut options = sides.clone();
        let legacy = take_legacy_flat_options(DrawingKind::RegressionTrend, &mut options, false);
        assert_eq!(legacy.regression_deviations, None);
        assert_eq!(options, sides);
        // A fork document's band also folds into one band at the wider enabled side, for
        // upstream's readers, and keeps its sides.
        let mut options = sides.clone();
        let legacy = take_legacy_flat_options(DrawingKind::RegressionTrend, &mut options, true);
        assert_eq!(legacy.regression_deviations, Some(3.0));
        assert_eq!(options, sides);
        // An omitted side is written as the fork's default, so the band no longer depends on the
        // fold; a symmetric band with both sides on is the fold alone.
        let mut options = serde_json::json!({"channel": {"upper_deviation": 3.0}});
        let legacy = take_legacy_flat_options(DrawingKind::RegressionTrend, &mut options, true);
        assert_eq!(legacy.regression_deviations, Some(3.0));
        assert_eq!(
            options,
            serde_json::json!({"channel": {"upper_deviation": 3.0, "lower_deviation": -2.0}})
        );
        let mut options = serde_json::json!({"channel": {
            "upper_deviation": 1.5, "lower_deviation": -1.5, "use_upper_deviation": true
        }});
        let legacy = take_legacy_flat_options(DrawingKind::RegressionTrend, &mut options, true);
        assert_eq!(legacy.regression_deviations, Some(1.5));
        assert_eq!(options, serde_json::json!({}));
        // Bars keep their list position as their offset; invalid bars drop out.
        let mut options = serde_json::json!({"projection_annotation": {
            "bars_mode": "hl_bars",
            "flipped": true,
            "bars": [[5.0, 6.0, 4.0, 5.5], [1.0, 0.5, 2.0, 1.0], [5.5, 8.0, 5.0, 7.0]]
        }});
        let legacy = take_legacy_flat_options(DrawingKind::BarsPattern, &mut options, false);
        assert_eq!(legacy.bars_pattern_mode.as_deref(), Some("bars"));
        assert_eq!(legacy.bars_pattern_mirror_x, None);
        assert_eq!(legacy.bars_pattern_mirror_y, Some(true));
        let offsets = legacy
            .bars_pattern
            .unwrap()
            .iter()
            .map(|bar| bar.offset)
            .collect::<Vec<_>>();
        assert_eq!(offsets, [0, 2]);
        assert_eq!(options, serde_json::json!({}));
        // An icon's size clamps into the upstream range.
        let mut options =
            serde_json::json!({"projection_annotation": {"icon": "heart", "icon_size": 120.0}});
        let legacy = take_legacy_flat_options(DrawingKind::IconStamp, &mut options, false);
        assert_eq!(legacy.icon_name.as_deref(), Some("heart"));
        assert_eq!(legacy.icon_size, Some(96.0));
        // A fork document's absent block reads as the block's defaults, and stays absent.
        let mut options = serde_json::json!({});
        let legacy = take_legacy_flat_options(DrawingKind::ElliottImpulse, &mut options, true);
        assert_eq!(legacy.wave_degree.as_deref(), Some("intermediate"));
        assert_eq!(options, serde_json::json!({}));
        let legacy = take_legacy_flat_options(DrawingKind::FibonacciTimeZones, &mut options, true);
        assert_eq!(legacy.level_reverse, Some(false));
        assert_eq!(legacy.level_show_values, Some(true));
        assert_eq!(legacy.level_show_percents, Some(false));
        // The fork labelled time levels right of their lines: upstream's `left`.
        assert_eq!(legacy.level_label_align.as_deref(), Some("left"));
        assert_eq!(
            legacy.level_log_scale, None,
            "time zones have no log levels"
        );
        let legacy = take_legacy_flat_options(DrawingKind::RegressionTrend, &mut options, true);
        assert_eq!(legacy.regression_deviations, Some(2.0));
        assert_eq!(options, serde_json::json!({}));
    }
}
