//! Thin browser boundary for engine-owned general Cartesian charts.

use aeris_charts_engine::{
    AxisDimension, AxisPosition, CategoryScaleType, ChartError, ContinuousScaleType,
    DEFAULT_GENERAL_FILL_OPACITY, GeneralAxisDomain, GeneralAxisOptions, GeneralAxisTick,
    GeneralBrushRange, GeneralBrushSnapshot, GeneralDatasetId, GeneralHitMode,
    GeneralInterpolation, GeneralLineStyle, GeneralPointSymbol, GeneralReferenceId,
    GeneralReferenceOptions, GeneralRowId, GeneralRowIdentity, GeneralScaleType, GeneralSeriesId,
    GeneralSeriesKind, GeneralSeriesOptions, GeneralStackMode, GeneralTooltipSnapshot,
    GeneralXyInput, HorizontalDomain, MAX_GENERAL_TEMPORAL_MILLISECONDS,
};
use js_sys::{Float64Array, Uint8Array, Uint32Array};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ChartInner;

#[derive(Deserialize)]
struct PaneInput {
    #[serde(default)]
    preserve_empty: bool,
    horizontal_domain: DomainInput,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DomainInput {
    FinancialTime,
    Continuous {
        #[serde(default)]
        scale: ContinuousInput,
    },
    Temporal,
    Category {
        #[serde(default)]
        scale: CategoryInput,
    },
    Polar,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContinuousInput {
    #[default]
    Linear,
    Log,
    Symlog,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CategoryInput {
    #[default]
    Band,
    Point,
}

#[derive(Deserialize)]
struct AxisInput {
    id: String,
    pane: usize,
    dimension: String,
    scale: String,
    #[serde(default)]
    position: Option<String>,
    #[serde(default)]
    domain: Option<Value>,
    #[serde(default)]
    reverse: bool,
    #[serde(default = "default_true")]
    visible: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tick_count: Option<u16>,
    #[serde(default)]
    ticks: Option<Vec<GeneralAxisTick>>,
    #[serde(default = "default_tick_gap")]
    min_tick_gap: f64,
    #[serde(default = "default_band_padding")]
    band_padding_inner: f64,
    #[serde(default = "default_band_padding")]
    band_padding_outer: f64,
    #[serde(default = "default_true")]
    zero_line: bool,
    #[serde(default = "default_true")]
    grid_visible: bool,
}

#[derive(Deserialize)]
struct SeriesInput {
    pane: usize,
    x_axis_id: String,
    y_axis_id: String,
    #[serde(default = "default_true")]
    visible: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default = "default_point_radius")]
    point_radius: f64,
    #[serde(default)]
    point_markers: bool,
    #[serde(default)]
    point_symbol: Option<String>,
    #[serde(default = "default_line_width")]
    line_width: f64,
    #[serde(default)]
    line_style: Option<String>,
    #[serde(default)]
    interpolation: Option<String>,
    #[serde(default)]
    connect_missing: bool,
    #[serde(default = "default_fill_opacity")]
    fill_opacity: f64,
    #[serde(default)]
    baseline_value: Option<f64>,
    #[serde(default)]
    data_labels: bool,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    stack_id: Option<String>,
    #[serde(default)]
    stack_mode: Option<String>,
}

#[derive(Deserialize)]
struct CategoryUpdateInput {
    categories: Vec<String>,
    max_rows: u32,
    labels: Option<Vec<Option<String>>>,
}

#[derive(Deserialize)]
struct CategoryDataInput {
    categories: Vec<String>,
    labels: Option<Vec<Option<String>>>,
}

#[derive(Deserialize)]
struct HeatmapCategoryDataInput {
    x_categories: Vec<String>,
    y_categories: Vec<String>,
    labels: Option<Vec<Option<String>>>,
}

#[derive(Deserialize)]
struct HeatmapCategoryUpdateInput {
    x_categories: Vec<String>,
    y_categories: Vec<String>,
    max_rows: u32,
    labels: Option<Vec<Option<String>>>,
}

#[derive(Deserialize)]
struct NumericDataInput {
    ids: Option<Vec<Value>>,
    labels: Option<Vec<Option<String>>>,
}

fn default_true() -> bool {
    true
}

fn default_tick_gap() -> f64 {
    4.0
}

fn default_band_padding() -> f64 {
    0.1
}

fn default_point_radius() -> f64 {
    3.0
}

fn default_line_width() -> f64 {
    2.0
}

fn default_fill_opacity() -> f64 {
    DEFAULT_GENERAL_FILL_OPACITY
}

fn line_style(value: Option<&str>) -> Result<GeneralLineStyle, ChartError> {
    match value.unwrap_or("solid") {
        "solid" => Ok(GeneralLineStyle::Solid),
        "dotted" => Ok(GeneralLineStyle::Dotted),
        "dashed" => Ok(GeneralLineStyle::Dashed),
        _ => Err(ChartError::new(
            aeris_charts_engine::ErrorCode::InvalidOptions,
            "general series line_style must be solid, dotted, or dashed",
        )),
    }
}

fn interpolation(value: Option<&str>) -> Result<GeneralInterpolation, ChartError> {
    match value.unwrap_or("linear") {
        "linear" => Ok(GeneralInterpolation::Linear),
        "step" => Ok(GeneralInterpolation::Step),
        "curved" => Ok(GeneralInterpolation::Curved),
        _ => Err(ChartError::new(
            aeris_charts_engine::ErrorCode::InvalidOptions,
            "general series interpolation must be linear, step, or curved",
        )),
    }
}

fn point_symbol(value: Option<&str>) -> Result<GeneralPointSymbol, ChartError> {
    match value.unwrap_or("circle") {
        "circle" => Ok(GeneralPointSymbol::Circle),
        "square" => Ok(GeneralPointSymbol::Square),
        "diamond" => Ok(GeneralPointSymbol::Diamond),
        "triangle" => Ok(GeneralPointSymbol::Triangle),
        _ => Err(ChartError::new(
            aeris_charts_engine::ErrorCode::InvalidOptions,
            "general series point_symbol must be circle, square, diamond, or triangle",
        )),
    }
}

fn series_options_from_input(
    kind: GeneralSeriesKind,
    dataset: GeneralDatasetId,
    input: SeriesInput,
) -> Result<GeneralSeriesOptions, ChartError> {
    let mut options = match kind {
        GeneralSeriesKind::XyLine => {
            GeneralSeriesOptions::xy_line(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::XyArea => {
            GeneralSeriesOptions::xy_area(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::RangeArea => {
            GeneralSeriesOptions::range_area(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::RangeBar => {
            GeneralSeriesOptions::range_bar(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::ErrorBar => {
            GeneralSeriesOptions::error_bar(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::Column => {
            GeneralSeriesOptions::column(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::HorizontalBar => GeneralSeriesOptions::horizontal_bar(
            input.pane,
            dataset,
            input.x_axis_id,
            input.y_axis_id,
        ),
        GeneralSeriesKind::BoxPlot => {
            GeneralSeriesOptions::box_plot(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::HeatmapGrid => GeneralSeriesOptions::heatmap_grid(
            input.pane,
            dataset,
            input.x_axis_id,
            input.y_axis_id,
        ),
        GeneralSeriesKind::Scatter => {
            GeneralSeriesOptions::scatter(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
        GeneralSeriesKind::Bubble => {
            GeneralSeriesOptions::bubble(input.pane, dataset, input.x_axis_id, input.y_axis_id)
        }
    };
    options.visible = input.visible;
    options.title = input.title;
    options.color = input.color;
    options.point_radius = input.point_radius;
    options.point_markers = input.point_markers;
    options.point_symbol = point_symbol(input.point_symbol.as_deref())?;
    options.line_width = input.line_width;
    options.line_style = line_style(input.line_style.as_deref())?;
    options.interpolation = interpolation(input.interpolation.as_deref())?;
    options.connect_missing = input.connect_missing;
    options.fill_opacity = input.fill_opacity;
    options.baseline_value = input.baseline_value;
    options.data_labels = input.data_labels;
    options.group_id = input.group_id;
    options.stack_id = input.stack_id;
    options.stack_mode = match input.stack_mode.as_deref().unwrap_or("normal") {
        "normal" => GeneralStackMode::Normal,
        "percent" => GeneralStackMode::Percent,
        _ => {
            return Err(ChartError::new(
                aeris_charts_engine::ErrorCode::InvalidOptions,
                "general series stack_mode must be normal or percent",
            ));
        }
    };
    Ok(options)
}

fn series_kind_name(kind: GeneralSeriesKind) -> &'static str {
    match kind {
        GeneralSeriesKind::XyLine => "xy_line",
        GeneralSeriesKind::XyArea => "xy_area",
        GeneralSeriesKind::RangeArea => "range_area",
        GeneralSeriesKind::RangeBar => "range_bar",
        GeneralSeriesKind::ErrorBar => "error_bar",
        GeneralSeriesKind::Column => "column",
        GeneralSeriesKind::HorizontalBar => "horizontal_bar",
        GeneralSeriesKind::BoxPlot => "box_plot",
        GeneralSeriesKind::HeatmapGrid => "heatmap_grid",
        GeneralSeriesKind::Scatter => "scatter",
        GeneralSeriesKind::Bubble => "bubble",
    }
}

fn result_ok(value: Value) -> String {
    json!({ "ok": true, "result": value }).to_string()
}

fn result_error(error: &ChartError) -> String {
    json!({
        "ok": false,
        "error": { "code": error.code().name(), "message": error.message() }
    })
    .to_string()
}

fn input_error(message: impl Into<String>) -> String {
    let error = ChartError::new(aeris_charts_engine::ErrorCode::InvalidOptions, message);
    result_error(&error)
}

fn data_error(message: impl Into<String>) -> String {
    let error = ChartError::new(aeris_charts_engine::ErrorCode::InvalidData, message);
    result_error(&error)
}

fn temporal_x_values(values: &Float64Array) -> Result<Vec<i64>, String> {
    values
        .to_vec()
        .into_iter()
        .map(|value| {
            if !value.is_finite()
                || value.fract() != 0.0
                || value.abs() > MAX_GENERAL_TEMPORAL_MILLISECONDS as f64
            {
                return Err(data_error(format!(
                    "general temporal X values must be whole epoch milliseconds within +/-{MAX_GENERAL_TEMPORAL_MILLISECONDS}"
                )));
            }
            Ok(value as i64)
        })
        .collect()
}

fn parse_json<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    serde_json::from_str(text)
        .map_err(|error| input_error(format!("invalid general-chart options: {error}")))
}

fn horizontal_domain(input: DomainInput) -> HorizontalDomain {
    match input {
        DomainInput::FinancialTime => HorizontalDomain::FinancialTime,
        DomainInput::Continuous { scale } => HorizontalDomain::Continuous {
            scale: match scale {
                ContinuousInput::Linear => ContinuousScaleType::Linear,
                ContinuousInput::Log => ContinuousScaleType::Logarithmic,
                ContinuousInput::Symlog => ContinuousScaleType::SymmetricLog,
            },
        },
        DomainInput::Temporal => HorizontalDomain::Temporal,
        DomainInput::Category { scale } => HorizontalDomain::Category {
            scale: match scale {
                CategoryInput::Band => CategoryScaleType::Band,
                CategoryInput::Point => CategoryScaleType::Point,
            },
        },
        DomainInput::Polar => HorizontalDomain::Polar,
    }
}

pub(super) fn parse_initial_horizontal_domain(
    options_json: &str,
) -> Result<HorizontalDomain, ChartError> {
    serde_json::from_str::<DomainInput>(options_json)
        .map(horizontal_domain)
        .map_err(|error| {
            ChartError::new(
                aeris_charts_engine::ErrorCode::InvalidOptions,
                format!("invalid initial horizontal domain: {error}"),
            )
        })
}

fn dimension(value: &str) -> Option<AxisDimension> {
    match value {
        "x" => Some(AxisDimension::X),
        "y" => Some(AxisDimension::Y),
        "angle" => Some(AxisDimension::Angle),
        "radius" => Some(AxisDimension::Radius),
        _ => None,
    }
}

fn position(value: Option<&str>) -> Option<Option<AxisPosition>> {
    match value {
        None => Some(None),
        Some("top") => Some(Some(AxisPosition::Top)),
        Some("bottom") => Some(Some(AxisPosition::Bottom)),
        Some("left") => Some(Some(AxisPosition::Left)),
        Some("right") => Some(Some(AxisPosition::Right)),
        Some(_) => None,
    }
}

fn scale(value: &str) -> Option<GeneralScaleType> {
    match value {
        "linear" => Some(GeneralScaleType::Linear),
        "log" => Some(GeneralScaleType::Logarithmic),
        "symlog" => Some(GeneralScaleType::SymmetricLog),
        "temporal" => Some(GeneralScaleType::Temporal),
        "band" => Some(GeneralScaleType::Band),
        "point" => Some(GeneralScaleType::Point),
        "radial_linear" => Some(GeneralScaleType::RadialLinear),
        "angular_category" => Some(GeneralScaleType::AngularCategory),
        _ => None,
    }
}

fn axis_domain(scale: GeneralScaleType, value: Option<Value>) -> Option<GeneralAxisDomain> {
    let Some(value) = value else {
        return Some(GeneralAxisDomain::Auto);
    };
    if value == "auto" {
        return Some(GeneralAxisDomain::Auto);
    }
    let values = value.as_array()?;
    match scale {
        GeneralScaleType::Linear
        | GeneralScaleType::Logarithmic
        | GeneralScaleType::SymmetricLog
        | GeneralScaleType::RadialLinear => {
            if values.len() != 2 {
                return None;
            }
            Some(GeneralAxisDomain::Numeric([
                values.first()?.as_f64()?,
                values.get(1)?.as_f64()?,
            ]))
        }
        GeneralScaleType::Temporal => {
            if values.len() != 2 {
                return None;
            }
            Some(GeneralAxisDomain::Temporal([
                values.first()?.as_i64()?,
                values.get(1)?.as_i64()?,
            ]))
        }
        GeneralScaleType::Band | GeneralScaleType::Point | GeneralScaleType::AngularCategory => {
            values
                .iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .map(GeneralAxisDomain::Category)
        }
    }
}

fn parse_ids(ids_json: &str) -> Result<Option<Vec<GeneralRowId>>, String> {
    if ids_json.is_empty() || ids_json == "null" {
        return Ok(None);
    }
    let values: Vec<Value> = serde_json::from_str(ids_json)
        .map_err(|error| input_error(format!("invalid general row IDs: {error}")))?;
    parse_id_values(Some(values))
}

fn parse_id_values(values: Option<Vec<Value>>) -> Result<Option<Vec<GeneralRowId>>, String> {
    let Some(values) = values else {
        return Ok(None);
    };
    values
        .into_iter()
        .map(|value| match value {
            Value::String(value) => Ok(GeneralRowId::Text(value)),
            Value::Number(value) => value
                .as_f64()
                .map(GeneralRowId::Number)
                .ok_or_else(|| input_error("general numeric row IDs must be finite numbers")),
            Value::Null => Ok(GeneralRowId::Generated),
            _ => Err(input_error(
                "general row IDs must be strings, numbers, or omitted-row markers",
            )),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub(super) fn row_identity(identity: &GeneralRowIdentity) -> Value {
    match identity {
        GeneralRowIdentity::Generated(value) => json!({ "generated": value.to_string() }),
        GeneralRowIdentity::Explicit(GeneralRowId::Number(value)) => json!(value),
        GeneralRowIdentity::Explicit(GeneralRowId::Text(value)) => json!(value),
        GeneralRowIdentity::Explicit(GeneralRowId::Generated) => Value::Null,
    }
}

pub(super) fn general_hit_value(hit: &aeris_charts_engine::GeneralSeriesHit) -> Value {
    json!({
        "series": hit.series.get(),
        "row": hit.row,
        "row_id": row_identity(&hit.row_id),
        "distance": hit.distance,
    })
}

fn general_tooltip_value(snapshot: GeneralTooltipSnapshot) -> Value {
    json!({
        "series": snapshot.series.get(), "row": snapshot.row,
        "row_id": row_identity(&snapshot.row_id), "x_label": snapshot.x_label,
        "y_label": snapshot.y_label,
        "label": snapshot.label, "value": snapshot.value, "low": snapshot.low, "high": snapshot.high,
        "x_low": snapshot.x_low, "x_high": snapshot.x_high,
        "q1": snapshot.q1, "q3": snapshot.q3,
        "size": snapshot.size, "title": snapshot.title,
    })
}

fn general_brush_value(snapshot: GeneralBrushSnapshot) -> Value {
    let range = match snapshot.range {
        GeneralBrushRange::Numeric([from, to]) => {
            json!({ "type": "numeric", "from": from, "to": to })
        }
        GeneralBrushRange::Temporal([from, to]) => {
            json!({ "type": "temporal", "from": from, "to": to })
        }
        GeneralBrushRange::Category([from, to]) => {
            json!({ "type": "category", "from": from, "to": to })
        }
    };
    json!({
        "pane": snapshot.pane,
        "axis_id": snapshot.axis_id,
        "dimension": match snapshot.dimension {
            AxisDimension::X => "x",
            AxisDimension::Y => "y",
            AxisDimension::Angle => "angle",
            AxisDimension::Radius => "radius",
        },
        "range": range,
        "items": snapshot.items.iter().map(general_hit_value).collect::<Vec<_>>(),
    })
}

impl ChartInner {
    pub fn add_general_pane_result_json(&mut self, options_json: &str) -> String {
        let input = match parse_json::<PaneInput>(options_json) {
            Ok(input) => input,
            Err(error) => return error,
        };
        match self.engine.add_pane_with_domain(
            input.preserve_empty,
            horizontal_domain(input.horizontal_domain),
        ) {
            Ok(index) => result_ok(json!({ "pane": index })),
            Err(error) => result_error(&error),
        }
    }

    pub fn add_general_axis_result_json(&mut self, options_json: &str) -> String {
        let input = match parse_json::<AxisInput>(options_json) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let Some(dimension) = dimension(&input.dimension) else {
            return input_error("unknown general axis dimension");
        };
        let Some(position) = position(input.position.as_deref()) else {
            return input_error("unknown general axis position");
        };
        let Some(scale) = scale(&input.scale) else {
            return input_error("unknown general axis scale");
        };
        let Some(domain) = axis_domain(scale, input.domain) else {
            return input_error("general axis domain does not match its scale");
        };
        let mut options = GeneralAxisOptions::new(input.id.clone(), input.pane, dimension, scale);
        options.position = position;
        options.domain = domain;
        options.reverse = input.reverse;
        options.visible = input.visible;
        options.title = input.title;
        options.tick_count = input.tick_count;
        options.ticks = input.ticks;
        options.min_tick_gap = input.min_tick_gap;
        options.band_padding_inner = input.band_padding_inner;
        options.band_padding_outer = input.band_padding_outer;
        options.zero_line = input.zero_line;
        options.grid_visible = input.grid_visible;
        match self.engine.add_general_axis(options) {
            Ok(()) => {
                let handle_token = self
                    .engine
                    .general_axis(&input.id)
                    .map_or(0, aeris_charts_engine::GeneralAxis::handle_token);
                result_ok(json!({ "id": input.id, "handle_token": handle_token }))
            }
            Err(error) => result_error(&error),
        }
    }

    pub fn update_general_axis_result_json(&mut self, options_json: &str) -> String {
        let input = match parse_json::<AxisInput>(options_json) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let Some(dimension) = dimension(&input.dimension) else {
            return input_error("unknown general axis dimension");
        };
        let Some(position) = position(input.position.as_deref()) else {
            return input_error("unknown general axis position");
        };
        let Some(scale) = scale(&input.scale) else {
            return input_error("unknown general axis scale");
        };
        let Some(domain) = axis_domain(scale, input.domain) else {
            return input_error("general axis domain does not match its scale");
        };
        let mut options = GeneralAxisOptions::new(input.id, input.pane, dimension, scale);
        options.position = position;
        options.domain = domain;
        options.reverse = input.reverse;
        options.visible = input.visible;
        options.title = input.title;
        options.tick_count = input.tick_count;
        options.ticks = input.ticks;
        options.min_tick_gap = input.min_tick_gap;
        options.band_padding_inner = input.band_padding_inner;
        options.band_padding_outer = input.band_padding_outer;
        options.zero_line = input.zero_line;
        options.grid_visible = input.grid_visible;
        match self.engine.update_general_axis_options(options) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn general_axis_handle_token(&self, id: &str) -> u32 {
        self.engine
            .general_axis(id)
            .map_or(0, aeris_charts_engine::GeneralAxis::handle_token)
    }

    pub fn general_axis_json(&self, id: &str) -> String {
        let Some(axis) = self.engine.general_axis(id) else {
            return "null".to_owned();
        };
        let domain = match axis.domain() {
            GeneralAxisDomain::Auto => json!("auto"),
            GeneralAxisDomain::Numeric(value) => json!(value),
            GeneralAxisDomain::Temporal(value) => json!(value),
            GeneralAxisDomain::Category(value) => json!(value),
        };
        let ticks = axis.ticks().map(|ticks| {
            ticks
                .iter()
                .map(|tick| match tick {
                    GeneralAxisTick::Numeric { value, label } => match label {
                        Some(label) => json!({ "type": "numeric", "value": value, "label": label }),
                        None => json!({ "type": "numeric", "value": value }),
                    },
                    GeneralAxisTick::Temporal { value, label } => match label {
                        Some(label) => {
                            json!({ "type": "temporal", "value": value, "label": label })
                        }
                        None => json!({ "type": "temporal", "value": value }),
                    },
                    GeneralAxisTick::Category { value, label } => match label {
                        Some(label) => {
                            json!({ "type": "category", "value": value, "label": label })
                        }
                        None => json!({ "type": "category", "value": value }),
                    },
                })
                .collect::<Vec<_>>()
        });
        json!({
            "id": axis.id(),
            "pane": self.engine.general_axis_pane_index(id),
            "dimension": match axis.dimension() { AxisDimension::X => "x", AxisDimension::Y => "y", AxisDimension::Angle => "angle", AxisDimension::Radius => "radius" },
            "position": axis.position().map(|value| match value { AxisPosition::Top => "top", AxisPosition::Bottom => "bottom", AxisPosition::Left => "left", AxisPosition::Right => "right" }),
            "scale": match axis.scale() { GeneralScaleType::Linear => "linear", GeneralScaleType::Logarithmic => "log", GeneralScaleType::SymmetricLog => "symlog", GeneralScaleType::Temporal => "temporal", GeneralScaleType::Band => "band", GeneralScaleType::Point => "point", GeneralScaleType::RadialLinear => "radial_linear", GeneralScaleType::AngularCategory => "angular_category" },
            "domain": domain,
            "reverse": axis.reverse(),
            "visible": axis.visible(),
            "title": axis.title(),
            "tick_count": axis.tick_count(),
            "ticks": ticks,
            "min_tick_gap": axis.min_tick_gap(),
            "band_padding_inner": axis.band_padding_inner(),
            "band_padding_outer": axis.band_padding_outer(),
            "zero_line": axis.zero_line(),
            "grid_visible": axis.grid_visible(),
        }).to_string()
    }

    pub fn general_axis_ids_json(&self, pane: i32) -> String {
        let pane = (pane >= 0).then_some(pane as usize);
        json!(
            self.engine
                .general_axes(pane)
                .into_iter()
                .map(|axis| axis.id())
                .collect::<Vec<_>>()
        )
        .to_string()
    }

    pub fn set_general_axis_visible(&mut self, id: &str, visible: bool) -> bool {
        self.engine.set_general_axis_visible(id, visible)
    }

    pub fn add_general_reference_result_json(&mut self, options_json: &str) -> String {
        let options = match parse_json::<GeneralReferenceOptions>(options_json) {
            Ok(options) => options,
            Err(error) => return error,
        };
        match self.engine.add_general_reference(options) {
            Ok(id) => result_ok(json!({ "id": id.get() })),
            Err(error) => result_error(&error),
        }
    }

    pub fn general_reference_options_json(&self, id: u32) -> String {
        let Some(id) = GeneralReferenceId::from_raw(id) else {
            return "null".to_owned();
        };
        self.engine
            .general_reference_options(id)
            .and_then(|options| serde_json::to_string(&options).ok())
            .unwrap_or_else(|| "null".to_owned())
    }

    pub fn general_reference_ids_json(&self, pane: i32) -> String {
        json!(
            self.engine
                .general_reference_ids((pane >= 0).then_some(pane as usize))
                .into_iter()
                .map(GeneralReferenceId::get)
                .collect::<Vec<_>>()
        )
        .to_string()
    }

    pub fn remove_general_reference(&mut self, id: u32) -> bool {
        GeneralReferenceId::from_raw(id).is_some_and(|id| self.engine.remove_general_reference(id))
    }

    pub fn general_series_ids(&self, pane: usize) -> Vec<u32> {
        self.engine
            .general_series_ids_in_pane(pane)
            .into_iter()
            .map(GeneralSeriesId::get)
            .collect()
    }

    pub fn general_series_order_json(&self, pane: i32) -> String {
        let ids = self
            .engine
            .general_series_order((pane >= 0).then_some(pane as usize))
            .into_iter()
            .map(GeneralSeriesId::get)
            .collect::<Vec<_>>();
        json!(ids).to_string()
    }

    pub fn set_general_series_order(&mut self, pane: i32, ids: Vec<u32>) -> bool {
        let Some(ids) = ids
            .into_iter()
            .map(GeneralSeriesId::from_raw)
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        self.engine
            .set_general_series_order((pane >= 0).then_some(pane as usize), ids)
    }

    /// Rehydrate browser handles after an atomic persistence restore.
    pub fn general_series_catalog_json(&self) -> String {
        let series = (0..self.engine.panes.len())
            .flat_map(|pane| self.engine.general_series_ids_in_pane(pane))
            .filter_map(|id| self.engine.general_series(id))
            .map(|series| {
                json!({
                    "id": series.id().get(),
                    "dataset": series.dataset().get(),
                    "x_axis_id": series.x_axis_id(),
                    "y_axis_id": series.y_axis_id(),
                    "kind": series_kind_name(series.kind()),
                })
            })
            .collect::<Vec<_>>();
        json!(series).to_string()
    }

    pub fn general_series_options_json(&self, series: u32) -> String {
        let Some(id) = GeneralSeriesId::from_raw(series) else {
            return "null".to_owned();
        };
        let Some(series) = self.engine.general_series(id) else {
            return "null".to_owned();
        };
        json!({
            "pane": self.engine.general_series_pane_index(id),
            "x_axis_id": series.x_axis_id(),
            "y_axis_id": series.y_axis_id(),
            "visible": series.visible(),
            "title": series.title(),
            "color": series.color(),
            "point_radius": series.point_radius(),
            "point_markers": series.point_markers(),
            "point_symbol": match series.point_symbol() {
                GeneralPointSymbol::Circle => "circle",
                GeneralPointSymbol::Square => "square",
                GeneralPointSymbol::Diamond => "diamond",
                GeneralPointSymbol::Triangle => "triangle",
            },
            "line_width": series.line_width(),
            "line_style": match series.line_style() {
                GeneralLineStyle::Solid => "solid",
                GeneralLineStyle::Dotted => "dotted",
                GeneralLineStyle::Dashed => "dashed",
            },
            "interpolation": match series.interpolation() {
                GeneralInterpolation::Linear => "linear",
                GeneralInterpolation::Step => "step",
                GeneralInterpolation::Curved => "curved",
            },
            "connect_missing": series.connect_missing(),
            "fill_opacity": series.fill_opacity(),
            "baseline_value": series.baseline_value(),
            "data_labels": series.data_labels(),
            "group_id": series.group_id(),
            "stack_id": series.stack_id(),
            "stack_mode": match series.stack_mode() {
                GeneralStackMode::Normal => "normal",
                GeneralStackMode::Percent => "percent",
            },
        })
        .to_string()
    }

    pub fn update_general_series_options_result_json(
        &mut self,
        series: u32,
        options_json: &str,
    ) -> String {
        let Some(id) = GeneralSeriesId::from_raw(series) else {
            return input_error("general series handle is stale");
        };
        let input = match parse_json::<SeriesInput>(options_json) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let Some(current) = self.engine.general_series(id) else {
            return input_error("general series handle is stale");
        };
        let (kind, dataset) = (current.kind(), current.dataset());
        let options = match series_options_from_input(kind, dataset, input) {
            Ok(options) => options,
            Err(error) => return result_error(&error),
        };
        match self.engine.update_general_series_options(id, options) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn general_legend_snapshot_json(&self, pane: i32) -> String {
        let snapshot = self
            .engine
            .general_legend_snapshot((pane >= 0).then_some(pane as usize));
        json!({
            "items": snapshot.items.into_iter().map(|item| {
                json!({
                    "series": item.series.get(),
                    "pane": item.pane,
                    "kind": match item.kind {
                        GeneralSeriesKind::XyLine => "xy_line",
                        GeneralSeriesKind::XyArea => "xy_area",
                        GeneralSeriesKind::RangeArea => "range_area",
                        GeneralSeriesKind::RangeBar => "range_bar",
                        GeneralSeriesKind::ErrorBar => "error_bar",
                        GeneralSeriesKind::Column => "column",
                        GeneralSeriesKind::HorizontalBar => "horizontal_bar",
                        GeneralSeriesKind::BoxPlot => "box_plot",
                        GeneralSeriesKind::HeatmapGrid => "heatmap_grid",
                        GeneralSeriesKind::Scatter => "scatter",
                        GeneralSeriesKind::Bubble => "bubble",
                    },
                    "title": item.title,
                    "color": item.color,
                    "visible": item.visible,
                })
            }).collect::<Vec<_>>()
        })
        .to_string()
    }

    pub fn add_general_series_result_json(&mut self, kind: &str, options_json: &str) -> String {
        let input = match parse_json::<SeriesInput>(options_json) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let (kind, empty) = match kind {
            "xy_line" | "xy_area" | "range_area" | "range_bar" => {
                let Some(domain) = self.engine.pane_horizontal_domain(input.pane) else {
                    return input_error(format!("{kind} references a stale pane"));
                };
                let is_range = matches!(kind, "range_area" | "range_bar");
                let empty = match (domain, is_range) {
                    (HorizontalDomain::Continuous { .. }, false) => GeneralXyInput::Numeric {
                        ids: None,
                        x: Vec::new(),
                        y: Vec::new(),
                        y_valid: None,
                    },
                    (HorizontalDomain::Continuous { .. }, true) => GeneralXyInput::RangeNumeric {
                        ids: None,
                        x: Vec::new(),
                        low: Vec::new(),
                        low_valid: None,
                        high: Vec::new(),
                        high_valid: None,
                    },
                    (HorizontalDomain::Temporal, false) => GeneralXyInput::Temporal {
                        ids: None,
                        x_epoch_ms: Vec::new(),
                        y: Vec::new(),
                        y_valid: None,
                    },
                    (HorizontalDomain::Temporal, true) => GeneralXyInput::RangeTemporal {
                        ids: None,
                        x_epoch_ms: Vec::new(),
                        low: Vec::new(),
                        low_valid: None,
                        high: Vec::new(),
                        high_valid: None,
                    },
                    (HorizontalDomain::Category { .. }, false) => GeneralXyInput::Category {
                        ids: None,
                        categories: Vec::new(),
                        category_indices: Vec::new(),
                        y: Vec::new(),
                        y_valid: None,
                    },
                    (HorizontalDomain::Category { .. }, true) => GeneralXyInput::RangeCategory {
                        ids: None,
                        categories: Vec::new(),
                        category_indices: Vec::new(),
                        low: Vec::new(),
                        low_valid: None,
                        high: Vec::new(),
                        high_valid: None,
                    },
                    (HorizontalDomain::FinancialTime | HorizontalDomain::Polar, _) => {
                        return input_error(format!(
                            "{kind} requires a continuous, temporal, or category pane"
                        ));
                    }
                };
                (
                    if kind == "xy_area" {
                        GeneralSeriesKind::XyArea
                    } else if kind == "range_area" {
                        GeneralSeriesKind::RangeArea
                    } else if kind == "range_bar" {
                        GeneralSeriesKind::RangeBar
                    } else {
                        GeneralSeriesKind::XyLine
                    },
                    empty,
                )
            }
            "column" => (
                GeneralSeriesKind::Column,
                GeneralXyInput::Category {
                    ids: None,
                    categories: Vec::new(),
                    category_indices: Vec::new(),
                    y: Vec::new(),
                    y_valid: None,
                },
            ),
            "horizontal_bar" => (
                GeneralSeriesKind::HorizontalBar,
                GeneralXyInput::Category {
                    ids: None,
                    categories: Vec::new(),
                    category_indices: Vec::new(),
                    y: Vec::new(),
                    y_valid: None,
                },
            ),
            "box_plot" => (
                GeneralSeriesKind::BoxPlot,
                GeneralXyInput::BoxCategory {
                    ids: None,
                    categories: Vec::new(),
                    category_indices: Vec::new(),
                    min: Vec::new(),
                    min_valid: None,
                    q1: Vec::new(),
                    q1_valid: None,
                    median: Vec::new(),
                    median_valid: None,
                    q3: Vec::new(),
                    q3_valid: None,
                    max: Vec::new(),
                    max_valid: None,
                },
            ),
            "heatmap_grid" => {
                let Some(domain) = self.engine.pane_horizontal_domain(input.pane) else {
                    return input_error("heatmap_grid references a stale pane");
                };
                let empty = match domain {
                    HorizontalDomain::Category { .. } => GeneralXyInput::HeatmapCategoryCategory {
                        ids: None,
                        x_categories: Vec::new(),
                        x_category_indices: Vec::new(),
                        y_categories: Vec::new(),
                        y_category_indices: Vec::new(),
                        value: Vec::new(),
                        value_valid: None,
                    },
                    HorizontalDomain::Continuous { .. } => GeneralXyInput::HeatmapNumericNumeric {
                        ids: None,
                        x: Vec::new(),
                        y_coordinate: Vec::new(),
                        value: Vec::new(),
                        value_valid: None,
                    },
                    HorizontalDomain::Temporal => GeneralXyInput::HeatmapTemporalNumeric {
                        ids: None,
                        x_epoch_ms: Vec::new(),
                        y_coordinate: Vec::new(),
                        value: Vec::new(),
                        value_valid: None,
                    },
                    HorizontalDomain::FinancialTime | HorizontalDomain::Polar => {
                        return input_error(
                            "heatmap_grid requires a category, continuous, or temporal pane",
                        );
                    }
                };
                (GeneralSeriesKind::HeatmapGrid, empty)
            }
            "scatter" => (
                GeneralSeriesKind::Scatter,
                GeneralXyInput::Numeric {
                    ids: None,
                    x: Vec::new(),
                    y: Vec::new(),
                    y_valid: None,
                },
            ),
            "bubble" => (
                GeneralSeriesKind::Bubble,
                GeneralXyInput::Bubble {
                    ids: None,
                    x: Vec::new(),
                    y: Vec::new(),
                    y_valid: None,
                    size: Vec::new(),
                    size_valid: None,
                },
            ),
            "error_bar" => {
                let Some(domain) = self.engine.pane_horizontal_domain(input.pane) else {
                    return input_error("error_bar references a stale pane");
                };
                let empty = match domain {
                    HorizontalDomain::Continuous { .. } => GeneralXyInput::ErrorNumeric {
                        ids: None,
                        x: Vec::new(),
                        y: Vec::new(),
                        y_valid: None,
                        x_low: Vec::new(),
                        x_low_valid: None,
                        x_high: Vec::new(),
                        x_high_valid: None,
                        y_low: Vec::new(),
                        y_low_valid: None,
                        y_high: Vec::new(),
                        y_high_valid: None,
                    },
                    HorizontalDomain::Temporal => GeneralXyInput::ErrorTemporal {
                        ids: None,
                        x_epoch_ms: Vec::new(),
                        y: Vec::new(),
                        y_valid: None,
                        x_low_epoch_ms: Vec::new(),
                        x_low_valid: None,
                        x_high_epoch_ms: Vec::new(),
                        x_high_valid: None,
                        y_low: Vec::new(),
                        y_low_valid: None,
                        y_high: Vec::new(),
                        y_high_valid: None,
                    },
                    HorizontalDomain::Category { .. } => GeneralXyInput::ErrorCategory {
                        ids: None,
                        categories: Vec::new(),
                        category_indices: Vec::new(),
                        y: Vec::new(),
                        y_valid: None,
                        y_low: Vec::new(),
                        y_low_valid: None,
                        y_high: Vec::new(),
                        y_high_valid: None,
                    },
                    _ => {
                        return input_error(
                            "error_bar requires a continuous, temporal, or category pane",
                        );
                    }
                };
                (GeneralSeriesKind::ErrorBar, empty)
            }
            _ => return input_error("unsupported general series kind"),
        };
        let dataset = match self.engine.create_general_xy_dataset(empty) {
            Ok(dataset) => dataset,
            Err(error) => return result_error(&error),
        };
        let options = match series_options_from_input(kind, dataset, input) {
            Ok(options) => options,
            Err(error) => {
                self.engine.remove_general_dataset(dataset);
                return result_error(&error);
            }
        };
        match self.engine.add_general_series(options) {
            Ok(series) => result_ok(json!({ "series": series.get(), "dataset": dataset.get() })),
            Err(error) => {
                self.engine.remove_general_dataset(dataset);
                result_error(&error)
            }
        }
    }

    pub fn set_general_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Numeric {
            ids,
            x: x.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_bubble_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        size: &Float64Array,
        size_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Bubble {
            ids,
            x: x.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            size: size.to_vec(),
            size_valid: size_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn set_general_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Temporal {
            ids,
            x_epoch_ms,
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn set_general_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &Uint32Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let metadata = match parse_json::<CategoryDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Category {
            ids,
            categories: metadata.categories,
            category_indices: category_indices.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_heatmap_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        x_category_indices: &Uint32Array,
        y_category_indices: &Uint32Array,
        value: &Float64Array,
        value_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let metadata = match parse_json::<HeatmapCategoryDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let input = GeneralXyInput::HeatmapCategoryCategory {
            ids,
            x_categories: metadata.x_categories,
            x_category_indices: x_category_indices.to_vec(),
            y_categories: metadata.y_categories,
            y_category_indices: y_category_indices.to_vec(),
            value: value.to_vec(),
            value_valid: value_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_heatmap_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::HeatmapNumericNumeric {
            ids,
            x: x.to_vec(),
            y_coordinate: y_coordinate.to_vec(),
            value: value.to_vec(),
            value_valid: value_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_heatmap_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let input = GeneralXyInput::HeatmapTemporalNumeric {
            ids,
            x_epoch_ms,
            y_coordinate: y_coordinate.to_vec(),
            value: value.to_vec(),
            value_valid: value_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_range_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        low: &Float64Array,
        low_valid: Option<Uint8Array>,
        high: &Float64Array,
        high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::RangeNumeric {
            ids,
            x: x.to_vec(),
            low: low.to_vec(),
            low_valid: low_valid.map(|values| values.to_vec()),
            high: high.to_vec(),
            high_valid: high_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_error_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        x_low: &Float64Array,
        x_low_valid: Option<Uint8Array>,
        x_high: &Float64Array,
        x_high_valid: Option<Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::ErrorNumeric {
            ids,
            x: x.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            x_low: x_low.to_vec(),
            x_low_valid: x_low_valid.map(|values| values.to_vec()),
            x_high: x_high.to_vec(),
            x_high_valid: x_high_valid.map(|values| values.to_vec()),
            y_low: y_low.to_vec(),
            y_low_valid: y_low_valid.map(|values| values.to_vec()),
            y_high: y_high.to_vec(),
            y_high_valid: y_high_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_range_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        low: &Float64Array,
        low_valid: Option<Uint8Array>,
        high: &Float64Array,
        high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let input = GeneralXyInput::RangeTemporal {
            ids,
            x_epoch_ms,
            low: low.to_vec(),
            low_valid: low_valid.map(|values| values.to_vec()),
            high: high.to_vec(),
            high_valid: high_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_error_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        x_low_epoch_ms: &Float64Array,
        x_low_valid: Option<Uint8Array>,
        x_high_epoch_ms: &Float64Array,
        x_high_valid: Option<Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let x_low_epoch_ms = match temporal_x_values(x_low_epoch_ms) {
            Ok(values) => values.into_iter().map(|value| value as f64).collect(),
            Err(error) => return error,
        };
        let x_high_epoch_ms = match temporal_x_values(x_high_epoch_ms) {
            Ok(values) => values.into_iter().map(|value| value as f64).collect(),
            Err(error) => return error,
        };
        let input = GeneralXyInput::ErrorTemporal {
            ids,
            x_epoch_ms,
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            x_low_epoch_ms,
            x_low_valid: x_low_valid.map(|values| values.to_vec()),
            x_high_epoch_ms,
            x_high_valid: x_high_valid.map(|values| values.to_vec()),
            y_low: y_low.to_vec(),
            y_low_valid: y_low_valid.map(|values| values.to_vec()),
            y_high: y_high.to_vec(),
            y_high_valid: y_high_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_range_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &Uint32Array,
        low: &Float64Array,
        low_valid: Option<Uint8Array>,
        high: &Float64Array,
        high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let metadata = match parse_json::<CategoryDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let input = GeneralXyInput::RangeCategory {
            ids,
            categories: metadata.categories,
            category_indices: category_indices.to_vec(),
            low: low.to_vec(),
            low_valid: low_valid.map(|values| values.to_vec()),
            high: high.to_vec(),
            high_valid: high_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_error_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &Uint32Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let metadata = match parse_json::<CategoryDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let input = GeneralXyInput::ErrorCategory {
            ids,
            categories: metadata.categories,
            category_indices: category_indices.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            y_low: y_low.to_vec(),
            y_low_valid: y_low_valid.map(|values| values.to_vec()),
            y_high: y_high.to_vec(),
            y_high_valid: y_high_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_general_box_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        metadata_json: &str,
        category_indices: &Uint32Array,
        min: &Float64Array,
        min_valid: Option<Uint8Array>,
        q1: &Float64Array,
        q1_valid: Option<Uint8Array>,
        median: &Float64Array,
        median_valid: Option<Uint8Array>,
        q3: &Float64Array,
        q3_valid: Option<Uint8Array>,
        max: &Float64Array,
        max_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let metadata = match parse_json::<CategoryDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let input = GeneralXyInput::BoxCategory {
            ids,
            categories: metadata.categories,
            category_indices: category_indices.to_vec(),
            min: min.to_vec(),
            min_valid: min_valid.map(|values| values.to_vec()),
            q1: q1.to_vec(),
            q1_valid: q1_valid.map(|values| values.to_vec()),
            median: median.to_vec(),
            median_valid: median_valid.map(|values| values.to_vec()),
            q3: q3.to_vec(),
            q3_valid: q3_valid.map(|values| values.to_vec()),
            max: max.to_vec(),
            max_valid: max_valid.map(|values| values.to_vec()),
        };
        match self
            .engine
            .replace_general_xy_dataset_labeled(dataset, input, metadata.labels)
        {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn upsert_general_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Numeric {
            ids,
            x: x.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_bubble_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        size: &Float64Array,
        size_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Bubble {
            ids,
            x: x.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            size: size.to_vec(),
            size_valid: size_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn upsert_general_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Temporal {
            ids,
            x_epoch_ms,
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn upsert_general_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &Uint32Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let update = match parse_json::<CategoryUpdateInput>(update_json) {
            Ok(update) => update,
            Err(error) => return error,
        };
        let input = GeneralXyInput::Category {
            ids,
            categories: update.categories,
            category_indices: category_indices.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            update.labels,
            (update.max_rows > 0).then_some(update.max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_heatmap_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        x_category_indices: &Uint32Array,
        y_category_indices: &Uint32Array,
        value: &Float64Array,
        value_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let update = match parse_json::<HeatmapCategoryUpdateInput>(update_json) {
            Ok(update) => update,
            Err(error) => return error,
        };
        let input = GeneralXyInput::HeatmapCategoryCategory {
            ids,
            x_categories: update.x_categories,
            x_category_indices: x_category_indices.to_vec(),
            y_categories: update.y_categories,
            y_category_indices: y_category_indices.to_vec(),
            value: value.to_vec(),
            value_valid: value_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            update.labels,
            (update.max_rows > 0).then_some(update.max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_heatmap_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::HeatmapNumericNumeric {
            ids,
            x: x.to_vec(),
            y_coordinate: y_coordinate.to_vec(),
            value: value.to_vec(),
            value_valid: value_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_heatmap_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y_coordinate: &Float64Array,
        value: &Float64Array,
        value_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let input = GeneralXyInput::HeatmapTemporalNumeric {
            ids,
            x_epoch_ms,
            y_coordinate: y_coordinate.to_vec(),
            value: value.to_vec(),
            value_valid: value_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_range_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        low: &Float64Array,
        low_valid: Option<Uint8Array>,
        high: &Float64Array,
        high_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::RangeNumeric {
            ids,
            x: x.to_vec(),
            low: low.to_vec(),
            low_valid: low_valid.map(|values| values.to_vec()),
            high: high.to_vec(),
            high_valid: high_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_error_numeric_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        x_low: &Float64Array,
        x_low_valid: Option<Uint8Array>,
        x_high: &Float64Array,
        x_high_valid: Option<Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let input = GeneralXyInput::ErrorNumeric {
            ids,
            x: x.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            x_low: x_low.to_vec(),
            x_low_valid: x_low_valid.map(|values| values.to_vec()),
            x_high: x_high.to_vec(),
            x_high_valid: x_high_valid.map(|values| values.to_vec()),
            y_low: y_low.to_vec(),
            y_low_valid: y_low_valid.map(|values| values.to_vec()),
            y_high: y_high.to_vec(),
            y_high_valid: y_high_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_range_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        low: &Float64Array,
        low_valid: Option<Uint8Array>,
        high: &Float64Array,
        high_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let input = GeneralXyInput::RangeTemporal {
            ids,
            x_epoch_ms,
            low: low.to_vec(),
            low_valid: low_valid.map(|values| values.to_vec()),
            high: high.to_vec(),
            high_valid: high_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_error_temporal_data_typed(
        &mut self,
        dataset: u32,
        metadata_json: &str,
        x_epoch_ms: &Float64Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        x_low_epoch_ms: &Float64Array,
        x_low_valid: Option<Uint8Array>,
        x_high_epoch_ms: &Float64Array,
        x_high_valid: Option<Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<Uint8Array>,
        max_rows: u32,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let metadata = match parse_json::<NumericDataInput>(metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => return error,
        };
        let ids = match parse_id_values(metadata.ids) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let x_epoch_ms = match temporal_x_values(x_epoch_ms) {
            Ok(values) => values,
            Err(error) => return error,
        };
        let x_low_epoch_ms = match temporal_x_values(x_low_epoch_ms) {
            Ok(values) => values.into_iter().map(|value| value as f64).collect(),
            Err(error) => return error,
        };
        let x_high_epoch_ms = match temporal_x_values(x_high_epoch_ms) {
            Ok(values) => values.into_iter().map(|value| value as f64).collect(),
            Err(error) => return error,
        };
        let input = GeneralXyInput::ErrorTemporal {
            ids,
            x_epoch_ms,
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            x_low_epoch_ms,
            x_low_valid: x_low_valid.map(|values| values.to_vec()),
            x_high_epoch_ms,
            x_high_valid: x_high_valid.map(|values| values.to_vec()),
            y_low: y_low.to_vec(),
            y_low_valid: y_low_valid.map(|values| values.to_vec()),
            y_high: y_high.to_vec(),
            y_high_valid: y_high_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            metadata.labels,
            (max_rows > 0).then_some(max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_range_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &Uint32Array,
        low: &Float64Array,
        low_valid: Option<Uint8Array>,
        high: &Float64Array,
        high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let update = match parse_json::<CategoryUpdateInput>(update_json) {
            Ok(update) => update,
            Err(error) => return error,
        };
        let input = GeneralXyInput::RangeCategory {
            ids,
            categories: update.categories,
            category_indices: category_indices.to_vec(),
            low: low.to_vec(),
            low_valid: low_valid.map(|values| values.to_vec()),
            high: high.to_vec(),
            high_valid: high_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            update.labels,
            (update.max_rows > 0).then_some(update.max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_error_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &Uint32Array,
        y: &Float64Array,
        y_valid: Option<Uint8Array>,
        y_low: &Float64Array,
        y_low_valid: Option<Uint8Array>,
        y_high: &Float64Array,
        y_high_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let update = match parse_json::<CategoryUpdateInput>(update_json) {
            Ok(update) => update,
            Err(error) => return error,
        };
        let input = GeneralXyInput::ErrorCategory {
            ids,
            categories: update.categories,
            category_indices: category_indices.to_vec(),
            y: y.to_vec(),
            y_valid: y_valid.map(|values| values.to_vec()),
            y_low: y_low.to_vec(),
            y_low_valid: y_low_valid.map(|values| values.to_vec()),
            y_high: y_high.to_vec(),
            y_high_valid: y_high_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            update.labels,
            (update.max_rows > 0).then_some(update.max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_general_box_category_data_typed(
        &mut self,
        dataset: u32,
        ids_json: &str,
        update_json: &str,
        category_indices: &Uint32Array,
        min: &Float64Array,
        min_valid: Option<Uint8Array>,
        q1: &Float64Array,
        q1_valid: Option<Uint8Array>,
        median: &Float64Array,
        median_valid: Option<Uint8Array>,
        q3: &Float64Array,
        q3_valid: Option<Uint8Array>,
        max: &Float64Array,
        max_valid: Option<Uint8Array>,
    ) -> String {
        let Some(dataset) = GeneralDatasetId::from_raw(dataset) else {
            return input_error("general dataset handle is stale");
        };
        let ids = match parse_ids(ids_json) {
            Ok(ids) => ids,
            Err(error) => return error,
        };
        let update = match parse_json::<CategoryUpdateInput>(update_json) {
            Ok(update) => update,
            Err(error) => return error,
        };
        let input = GeneralXyInput::BoxCategory {
            ids,
            categories: update.categories,
            category_indices: category_indices.to_vec(),
            min: min.to_vec(),
            min_valid: min_valid.map(|values| values.to_vec()),
            q1: q1.to_vec(),
            q1_valid: q1_valid.map(|values| values.to_vec()),
            median: median.to_vec(),
            median_valid: median_valid.map(|values| values.to_vec()),
            q3: q3.to_vec(),
            q3_valid: q3_valid.map(|values| values.to_vec()),
            max: max.to_vec(),
            max_valid: max_valid.map(|values| values.to_vec()),
        };
        match self.engine.upsert_general_xy_dataset_labeled(
            dataset,
            input,
            update.labels,
            (update.max_rows > 0).then_some(update.max_rows as usize),
        ) {
            Ok(()) => result_ok(Value::Null),
            Err(error) => result_error(&error),
        }
    }

    pub fn remove_general_series(&mut self, series: u32, dataset: u32) -> bool {
        let (Some(series), Some(dataset)) = (
            GeneralSeriesId::from_raw(series),
            GeneralDatasetId::from_raw(dataset),
        ) else {
            return false;
        };
        self.engine.remove_general_series(series) && self.engine.remove_general_dataset(dataset)
    }

    pub fn set_general_series_visible(&mut self, series: u32, visible: bool) -> bool {
        GeneralSeriesId::from_raw(series)
            .is_some_and(|series| self.engine.set_general_series_visible(series, visible))
    }

    pub fn general_tooltip_json(&self, series: u32, row: usize) -> String {
        let Some(series) = GeneralSeriesId::from_raw(series) else {
            return "null".to_owned();
        };
        let Some(snapshot) = self.engine.general_tooltip_snapshot(series, row) else {
            return "null".to_owned();
        };
        general_tooltip_value(snapshot).to_string()
    }

    pub fn general_shared_tooltip_json(&self, series: u32, row: usize) -> String {
        let Some(series) = GeneralSeriesId::from_raw(series) else {
            return "null".to_owned();
        };
        let Some(snapshot) = self.engine.general_shared_tooltip_snapshot(series, row) else {
            return "null".to_owned();
        };
        json!({
            "pane": snapshot.pane,
            "anchor_series": snapshot.anchor_series.get(),
            "anchor_row": snapshot.anchor_row,
            "items": snapshot.items.into_iter().map(general_tooltip_value).collect::<Vec<_>>(),
        })
        .to_string()
    }

    pub fn set_general_brush_result_json(
        &mut self,
        axis_id: &str,
        from_css: f64,
        to_css: f64,
    ) -> String {
        match self
            .engine
            .set_general_brush_from_pixels(axis_id, from_css, to_css)
        {
            Ok(snapshot) => result_ok(general_brush_value(snapshot)),
            Err(error) => result_error(&error),
        }
    }

    pub fn general_brush_snapshot_json(&self) -> String {
        self.engine.general_brush_snapshot().map_or_else(
            || "null".to_owned(),
            |snapshot| general_brush_value(snapshot).to_string(),
        )
    }

    pub fn clear_general_brush(&mut self) {
        self.engine.clear_general_brush();
    }

    pub fn general_accessibility_json(&self, series: u32, offset: usize, limit: usize) -> String {
        let Some(series) = GeneralSeriesId::from_raw(series) else {
            return "null".to_owned();
        };
        let Some(snapshot) = self
            .engine
            .general_accessibility_snapshot(series, offset, limit)
        else {
            return "null".to_owned();
        };
        json!({
            "series": snapshot.series.get(),
            "title": snapshot.title,
            "total_rows": snapshot.total_rows,
            "offset": snapshot.offset,
            "items": snapshot.items.into_iter().map(|item| json!({
                "row": item.row,
                "row_id": row_identity(&item.row_id),
                "x_label": item.x_label,
                "y_label": item.y_label,
                "label": item.label,
                "value": item.value,
                "low": item.low,
                "high": item.high,
                "x_low": item.x_low,
                "x_high": item.x_high,
                "q1": item.q1,
                "q3": item.q3,
                "size": item.size,
            })).collect::<Vec<_>>(),
        })
        .to_string()
    }

    pub fn general_hit_test_json(&self, pane: usize, x: f64, y: f64, max_distance: f64) -> String {
        let mode = if max_distance < 0.0 {
            GeneralHitMode::Exact
        } else {
            GeneralHitMode::Nearest { max_distance }
        };
        let Some(hit) = self.engine.general_hit_test(pane, x, y, mode) else {
            return "null".to_owned();
        };
        general_hit_value(&hit).to_string()
    }

    pub fn general_selected_hit_json(&self) -> String {
        self.engine.general_selected_hit().as_ref().map_or_else(
            || "null".to_owned(),
            |hit| general_hit_value(hit).to_string(),
        )
    }

    pub fn general_accessibility_focused_hit_json(&self) -> String {
        self.engine
            .general_accessibility_focused_hit()
            .as_ref()
            .map_or_else(
                || "null".to_owned(),
                |hit| general_hit_value(hit).to_string(),
            )
    }

    pub fn select_general_hovered(&mut self) -> bool {
        self.engine.select_general_hovered()
    }

    pub fn clear_general_selection(&mut self) {
        self.engine.clear_general_selection();
    }

    pub fn set_general_accessibility_focus(&mut self, series: u32, row: usize) -> bool {
        let Some(series) = GeneralSeriesId::from_raw(series) else {
            return false;
        };
        self.engine.set_general_accessibility_focus(series, row)
    }

    pub fn clear_general_accessibility_focus(&mut self) {
        self.engine.clear_general_accessibility_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_axis_and_id_inputs_parse_without_host_state() {
        let pane: PaneInput =
            serde_json::from_str(r#"{"horizontal_domain":{"type":"continuous","scale":"symlog"}}"#)
                .unwrap();
        assert_eq!(
            horizontal_domain(pane.horizontal_domain),
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::SymmetricLog
            }
        );
        assert_eq!(parse_ids(r#"[1,"row"]"#).unwrap().unwrap().len(), 2);
        assert!(axis_domain(GeneralScaleType::Linear, Some(json!([0.0, 1.0]))).is_some());
        assert!(axis_domain(GeneralScaleType::Linear, Some(json!(["bad", 1.0]))).is_none());
    }

    #[test]
    fn general_line_styles_are_portable_and_reject_unknown_values() {
        assert_eq!(line_style(None).unwrap(), GeneralLineStyle::Solid);
        assert_eq!(
            line_style(Some("dotted")).unwrap(),
            GeneralLineStyle::Dotted
        );
        assert_eq!(
            line_style(Some("dashed")).unwrap(),
            GeneralLineStyle::Dashed
        );
        assert!(line_style(Some("large_dashed")).is_err());
        assert_eq!(interpolation(None).unwrap(), GeneralInterpolation::Linear);
        assert_eq!(
            interpolation(Some("step")).unwrap(),
            GeneralInterpolation::Step
        );
        assert_eq!(
            interpolation(Some("curved")).unwrap(),
            GeneralInterpolation::Curved
        );
        assert!(interpolation(Some("basis")).is_err());
        assert_eq!(point_symbol(None).unwrap(), GeneralPointSymbol::Circle);
        assert_eq!(
            point_symbol(Some("square")).unwrap(),
            GeneralPointSymbol::Square
        );
        assert_eq!(
            point_symbol(Some("diamond")).unwrap(),
            GeneralPointSymbol::Diamond
        );
        assert_eq!(
            point_symbol(Some("triangle")).unwrap(),
            GeneralPointSymbol::Triangle
        );
        assert!(point_symbol(Some("star")).is_err());
    }
}
