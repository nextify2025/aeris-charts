//! Chart-local, engine-scheduled custom study definitions and runtimes.

use crate::indicators::{OutputRows, price_input, store_indicator_output};
use crate::*;
use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;

pub const MAX_CUSTOM_STUDY_TYPES: usize = 64;
pub const MAX_CUSTOM_STUDY_BINDINGS: usize = 32;
pub const MAX_CUSTOM_STUDY_OUTPUTS: usize = 5;
pub const MAX_CUSTOM_STUDY_FAULTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomStudyPlot {
    Line,
    Histogram,
    Area,
    Marker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomStudyPane {
    Price,
    Dedicated,
}

#[derive(Clone, Debug)]
pub struct CustomStudyOutput {
    pub name: String,
    pub plot: CustomStudyPlot,
    pub pane: CustomStudyPane,
    pub default_style: IndicatorOutputStyle,
}

#[derive(Clone, Debug)]
pub struct CustomStudyDefinition {
    pub type_id: String,
    pub version: u32,
    pub title: String,
    pub parameters: Vec<IndicatorParameterDescriptor>,
    pub outputs: Vec<CustomStudyOutput>,
    pub uses_volume: bool,
}

pub type CustomStudyParams = BTreeMap<String, serde_json::Value>;

/// The rows a custom runtime computes: every column covers the source through its last real row
/// (trailing whitespace session slots are excluded), and `close` is the binding's selected input.
pub struct CustomStudyInput<'a> {
    pub times: &'a [i64],
    pub open: &'a [f64],
    pub high: &'a [f64],
    pub low: &'a [f64],
    pub close: &'a [f64],
    /// Volume aligned to `times` by timestamp, exactly `times.len()` long (`NaN` where the volume
    /// series has no row, including rows past its end), or empty without a volume source.
    pub volume: &'a [f64],
    pub from: usize,
    pub tail: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomStudyFault {
    pub message: String,
}

impl From<&str> for CustomStudyFault {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomStudyFaultEvent {
    pub binding: SeriesId,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustomStudyStats {
    pub calls: u64,
    pub rows: u64,
}

pub trait CustomStudyRuntime {
    fn compute(
        &mut self,
        input: CustomStudyInput<'_>,
        out: &mut [Vec<f64>],
    ) -> Result<(), CustomStudyFault>;
}

pub type CustomStudyFactory =
    Box<dyn Fn(&CustomStudyParams) -> Result<Box<dyn CustomStudyRuntime>, CustomStudyFault>>;

pub(crate) struct RegisteredCustomStudy {
    pub definition: CustomStudyDefinition,
    pub factory: CustomStudyFactory,
}

pub(crate) enum CustomBindingState {
    Pending,
    Active {
        runtime: Box<dyn CustomStudyRuntime>,
        covered: usize,
    },
    Faulted(String),
}

pub(crate) struct CustomBinding {
    pub state: CustomBindingState,
    pub stats: CustomStudyStats,
}

fn valid_value(descriptor: &IndicatorParameterDescriptor, value: &serde_json::Value) -> bool {
    let number = match descriptor.parameter_type {
        IndicatorParameterType::Integer => {
            let Some(n) = value.as_f64() else {
                return false;
            };
            if n.fract() != 0.0 || n.abs() > MAX_SAFE_VALUE {
                return false;
            }
            Some(n)
        }
        IndicatorParameterType::Number => value.as_f64(),
        IndicatorParameterType::Boolean => return value.is_boolean(),
        IndicatorParameterType::Choice => {
            return value.as_str().is_some_and(|s| {
                descriptor
                    .options
                    .as_ref()
                    .is_some_and(|options| options.iter().any(|o| o == s))
            });
        }
        _ => return false,
    };
    number.is_some_and(|n| {
        n.is_finite()
            && descriptor.min.is_none_or(|min| n >= min)
            && descriptor.max.is_none_or(|max| n <= max)
    })
}

pub(crate) fn normalize_params(
    definition: &CustomStudyDefinition,
    supplied: &CustomStudyParams,
) -> Result<CustomStudyParams, ChartError> {
    if supplied
        .keys()
        .any(|key| !definition.parameters.iter().any(|p| &p.name == key))
    {
        return Err(ChartError::new(
            ErrorCode::InvalidOptions,
            "unknown custom study parameter",
        ));
    }
    let mut params = CustomStudyParams::new();
    for p in &definition.parameters {
        let value = supplied.get(&p.name).unwrap_or(&p.default);
        if !valid_value(p, value) {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "invalid custom study parameter",
            ));
        }
        params.insert(p.name.clone(), value.clone());
    }
    Ok(params)
}

fn validate_definition(def: &CustomStudyDefinition) -> Result<(), ChartError> {
    let invalid = || ChartError::new(ErrorCode::InvalidOptions, "invalid custom study definition");
    if !valid_custom_type_id(&def.type_id)
        || def.version == 0
        || def.title.is_empty()
        || def.title.len() > 256
        || !(1..=MAX_CUSTOM_STUDY_OUTPUTS).contains(&def.outputs.len())
        || def.outputs.iter().any(|o| {
            o.name.is_empty()
                || o.name.len() > 128
                || !o
                    .default_style
                    .line_width
                    .is_none_or(|w| w.is_finite() && w > 0.0)
                || o.default_style.line_style > 4
                || [
                    &o.default_style.line_color,
                    &o.default_style.up_color,
                    &o.default_style.down_color,
                    &o.default_style.area_top_color,
                    &o.default_style.area_bottom_color,
                ]
                .iter()
                .any(|color| {
                    color.as_ref().is_some_and(|c| {
                        c.len() > 256 || aeris_charts_render::color::Color::parse_css(c).is_none()
                    })
                })
        })
        || def.parameters.len() > 64
    {
        return Err(invalid());
    }
    let mut names = std::collections::BTreeSet::new();
    for p in &def.parameters {
        if p.name.is_empty()
            || p.name.len() > 64
            || !names.insert(&p.name)
            || p.min.is_some_and(|n| !n.is_finite())
            || p.max.is_some_and(|n| !n.is_finite())
            || matches!((p.min, p.max), (Some(a), Some(b)) if a > b)
            || (p.parameter_type == IndicatorParameterType::Choice) != p.options.is_some()
            || p.options.as_ref().is_some_and(|options| {
                options.is_empty()
                    || options.len() > 64
                    || options.iter().any(|o| o.is_empty() || o.len() > 128)
                    || options
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != options.len()
            })
            || (p.parameter_type != IndicatorParameterType::Choice && p.options.is_some())
            || (matches!(
                p.parameter_type,
                IndicatorParameterType::Boolean | IndicatorParameterType::Choice
            ) && (p.min.is_some() || p.max.is_some()))
            || !valid_value(p, &p.default)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) fn valid_custom_type_id(type_id: &str) -> bool {
    !type_id.is_empty()
        && type_id.len() <= 64
        && type_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

impl ChartEngine {
    pub fn custom_study_schema(&self, type_id: &str) -> Option<IndicatorSchema> {
        let definition = &self.custom_studies.get(type_id)?.definition;
        Some(IndicatorSchema {
            revision: INDICATOR_SCHEMA_REVISION,
            kind: type_id.to_string(),
            parameters: definition.parameters.clone(),
            outputs: definition
                .outputs
                .iter()
                .enumerate()
                .map(|(index, output)| IndicatorOutputDescriptor {
                    name: output.name.clone(),
                    index,
                    supports_style: true,
                })
                .collect(),
        })
    }

    pub fn register_custom_study(
        &mut self,
        definition: CustomStudyDefinition,
        factory: CustomStudyFactory,
    ) -> Result<(), ChartError> {
        validate_definition(&definition)?;
        if self.custom_studies.contains_key(&definition.type_id) {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "custom study type already registered",
            ));
        }
        if self.custom_studies.len() == MAX_CUSTOM_STUDY_TYPES {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                "custom study type limit",
            ));
        }
        let type_id = definition.type_id.clone();
        let version = definition.version;
        self.custom_studies.insert(
            type_id.clone(),
            RegisteredCustomStudy {
                definition,
                factory,
            },
        );
        let pending = self.indicators.iter().filter(|b| matches!(
            &b.kind, IndicatorKind::Custom { type_id: id, version: v, .. } if id == &type_id && *v == version
        ) && matches!(b.runtime.custom().map(|c| &c.state), Some(CustomBindingState::Pending)))
            .map(|b| b.outputs[0]).collect::<Vec<_>>();
        for binding in pending {
            if let Some(outputs) = self
                .indicators
                .iter()
                .find(|b| b.outputs.first() == Some(&binding))
                .map(|b| b.outputs.clone())
            {
                let definition = self.custom_studies[&type_id].definition.clone();
                let source_pane = self
                    .indicators
                    .iter()
                    .find(|b| b.outputs[0] == binding)
                    .and_then(|b| self.series_entry(b.source))
                    .map(|s| s.pane_index);
                let mut dedicated = Vec::new();
                for (&output, descriptor) in outputs.iter().zip(&definition.outputs) {
                    if let Some(series) = self.series_entry_mut(output) {
                        series.title = format!("{} {}", definition.title, descriptor.name);
                    }
                    match descriptor.plot {
                        CustomStudyPlot::Histogram => {
                            self.convert_series_kind(output, SeriesKind::Histogram);
                        }
                        CustomStudyPlot::Area => {
                            self.convert_series_kind(output, SeriesKind::Area);
                        }
                        _ => {}
                    }
                    if descriptor.pane == CustomStudyPane::Dedicated
                        && source_pane == self.series_entry(output).map(|s| s.pane_index)
                    {
                        dedicated.push(output);
                    }
                }
                if !dedicated.is_empty() {
                    self.place_outputs_in_oscillator_pane(&dedicated);
                }
            }
            let _ = self.retry_custom_study(binding);
        }
        Ok(())
    }

    pub fn add_custom_study(
        &mut self,
        type_id: &str,
        source: SeriesId,
        source_input: IndicatorInputSource,
        volume: Option<SeriesId>,
        params: CustomStudyParams,
    ) -> Result<Vec<SeriesId>, ChartError> {
        let def = &self
            .custom_studies
            .get(type_id)
            .ok_or_else(|| ChartError::new(ErrorCode::InvalidOptions, "unknown custom study type"))?
            .definition;
        let params = normalize_params(def, &params)?;
        let kind = IndicatorKind::Custom {
            type_id: type_id.into(),
            version: def.version,
            parameters: params,
            output_count: def.outputs.len(),
        };
        self.create_custom_binding(source, source_input, kind, volume, false, &[])
    }

    pub fn restore_custom_study(
        &mut self,
        source: SeriesId,
        source_input: IndicatorInputSource,
        kind: IndicatorKind,
        volume_source: Option<SeriesId>,
    ) -> Result<Vec<SeriesId>, ChartError> {
        self.restore_custom_study_with_panes(source, source_input, kind, volume_source, &[])
    }

    pub(crate) fn restore_custom_study_with_panes(
        &mut self,
        source: SeriesId,
        source_input: IndicatorInputSource,
        kind: IndicatorKind,
        volume_source: Option<SeriesId>,
        dedicated_outputs: &[bool],
    ) -> Result<Vec<SeriesId>, ChartError> {
        self.create_custom_binding(
            source,
            source_input,
            kind,
            volume_source,
            true,
            dedicated_outputs,
        )
    }

    fn create_custom_binding(
        &mut self,
        source: SeriesId,
        source_input: IndicatorInputSource,
        kind: IndicatorKind,
        volume: Option<SeriesId>,
        restore: bool,
        dedicated_outputs: &[bool],
    ) -> Result<Vec<SeriesId>, ChartError> {
        let IndicatorKind::Custom {
            type_id,
            version,
            parameters,
            output_count,
        } = &kind
        else {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "not a custom study",
            ));
        };
        if self
            .indicators
            .iter()
            .filter(|b| b.runtime.custom().is_some())
            .count()
            >= MAX_CUSTOM_STUDY_BINDINGS
        {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                "custom study binding limit",
            ));
        }
        if !(1..=MAX_CUSTOM_STUDY_OUTPUTS).contains(output_count)
            || self.series_entry(source).is_none()
            || volume.is_some_and(|id| {
                id == source
                    || self
                        .series_entry(id)
                        .is_none_or(|s| !s.kind.stores_scalar_values())
            })
        {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "invalid custom study source or outputs",
            ));
        }
        let def = self
            .custom_studies
            .get(type_id)
            .filter(|d| d.definition.version == *version)
            .map(|d| d.definition.clone());
        if let Some(ref def) = def {
            if def.outputs.len() != *output_count || (volume.is_some() && !def.uses_volume) {
                return Err(ChartError::new(
                    ErrorCode::InvalidOptions,
                    "custom study output or volume mismatch",
                ));
            }
            if normalize_params(def, parameters)? != *parameters {
                return Err(ChartError::new(
                    ErrorCode::InvalidOptions,
                    "custom study parameters are not normalized",
                ));
            }
        } else if !restore {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "unknown custom study version",
            ));
        }
        let ids = (0..*output_count)
            .map(|_| self.add_series(SeriesKind::Line))
            .collect::<Vec<_>>();
        let placement = self
            .series_entry(source)
            .map(|s| (s.pane_index, s.price_scale_target));
        let mut dedicated = Vec::new();
        for (i, &id) in ids.iter().enumerate() {
            let descriptor = def.as_ref().and_then(|d| d.outputs.get(i));
            let title = descriptor.map_or_else(
                || format!("{type_id} {}", i + 1),
                |o| format!("{} {}", def.as_ref().unwrap().title, o.name),
            );
            if let Some(s) = self.series_entry_mut(id) {
                s.title = title;
                s.countdown_visible = false;
                s.last_price_animation = false;
                if let Some((pane, scale)) = placement {
                    s.pane_index = pane;
                    s.price_scale_target = scale;
                }
            }
            if let Some(o) = descriptor {
                match o.plot {
                    CustomStudyPlot::Histogram => {
                        self.convert_series_kind(id, SeriesKind::Histogram);
                    }
                    CustomStudyPlot::Area => {
                        self.convert_series_kind(id, SeriesKind::Area);
                    }
                    _ => {}
                }
                let _ = self.set_custom_output_style_before_binding(id, &o.default_style);
            }
            if dedicated_outputs
                .get(i)
                .copied()
                .unwrap_or_else(|| descriptor.is_some_and(|o| o.pane == CustomStudyPane::Dedicated))
            {
                dedicated.push(id);
            }
        }
        if !dedicated.is_empty() {
            self.place_outputs_in_oscillator_pane(&dedicated);
        }
        self.indicators.push(IndicatorBinding {
            source,
            source_input,
            kind,
            outputs: ids.clone(),
            volume_source: volume,
            annotations: None,
            structure: None,
            session: None,
            calendar: None,
            amount_source: None,
            runtime: super::indicators::BindingRuntime::Custom(CustomBinding {
                state: CustomBindingState::Pending,
                stats: CustomStudyStats::default(),
            }),
            inputs: Default::default(),
            source_generation: 0,
            volume_generation: None,
            amount_generation: None,
            data_end: 0,
        });
        let changes = self.rebuild_indicator(self.indicators.len() - 1, 0, true);
        self.indicator_changes.clear();
        self.indicator_changes.extend(changes.into_iter().flatten());
        self.propagate_indicator_changes();
        self.sync_time_points();
        Ok(ids)
    }

    fn set_custom_output_style_before_binding(
        &mut self,
        id: SeriesId,
        style: &IndicatorOutputStyle,
    ) -> bool {
        if let Some(s) = self.series_entry_mut(id) {
            s.visible = style.visible;
            s.line_color = style.line_color.clone();
            s.line_width = style.line_width;
            s.line_style = style.line_style;
            s.point_markers = style.point_markers;
            s.up_color = style.up_color.clone();
            s.down_color = style.down_color.clone();
            s.area_top_color = style.area_top_color.clone();
            s.area_bottom_color = style.area_bottom_color.clone();
            true
        } else {
            false
        }
    }

    pub fn custom_study_is_resolved(&self, binding: SeriesId) -> bool {
        self.indicators
            .iter()
            .find(|b| b.outputs.first() == Some(&binding))
            .and_then(|b| b.runtime.custom())
            .is_some_and(|c| !matches!(c.state, CustomBindingState::Pending))
    }

    pub fn custom_study_stats(&self, binding: SeriesId) -> Option<CustomStudyStats> {
        self.indicators
            .iter()
            .find(|b| b.outputs.first() == Some(&binding))
            .and_then(|b| b.runtime.custom())
            .map(|c| c.stats)
    }

    /// Drain the queued fault events (at most [`MAX_CUSTOM_STUDY_FAULTS`], oldest dropped first).
    /// ponytail: Rust hosts poll this; add a GPUI push hook only if Aeris Terminal needs push
    /// delivery.
    pub fn take_custom_study_faults(&mut self) -> Vec<CustomStudyFaultEvent> {
        self.custom_study_faults.drain(..).collect()
    }

    pub fn retry_custom_study(&mut self, binding: SeriesId) -> Result<(), ChartError> {
        let index = self
            .indicators
            .iter()
            .position(|b| b.outputs.first() == Some(&binding) && b.runtime.custom().is_some())
            .ok_or_else(|| {
                ChartError::new(ErrorCode::InvalidHandle, "unknown custom study binding")
            })?;
        self.indicators[index].runtime.custom_mut().unwrap().state = CustomBindingState::Pending;
        self.indicator_changes.clear();
        let changes = self.rebuild_indicator(index, 0, true);
        self.indicator_changes.extend(changes.into_iter().flatten());
        self.propagate_indicator_changes();
        self.sync_time_points();
        Ok(())
    }

    pub fn set_custom_study_parameters(
        &mut self,
        binding: SeriesId,
        supplied: CustomStudyParams,
    ) -> Result<(), ChartError> {
        let index = self
            .indicators
            .iter()
            .position(|b| b.outputs.first() == Some(&binding) && b.runtime.custom().is_some())
            .ok_or_else(|| {
                ChartError::new(ErrorCode::InvalidHandle, "unknown custom study binding")
            })?;
        let IndicatorKind::Custom {
            type_id, version, ..
        } = &self.indicators[index].kind
        else {
            unreachable!()
        };
        let def = self
            .custom_studies
            .get(type_id)
            .filter(|d| d.definition.version == *version)
            .ok_or_else(|| {
                ChartError::new(
                    ErrorCode::InvalidOptions,
                    "unregistered custom study version",
                )
            })?;
        let params = normalize_params(&def.definition, &supplied)?;
        if let IndicatorKind::Custom { parameters, .. } = &mut self.indicators[index].kind {
            *parameters = params;
        }
        self.retry_custom_study(binding)
    }

    /// Rebind a custom study without changing its output identities or dependency order.
    pub fn set_custom_study_source(
        &mut self,
        binding: SeriesId,
        source: SeriesId,
        source_input: IndicatorInputSource,
        volume: Option<SeriesId>,
    ) -> Result<(), ChartError> {
        let index = self
            .indicators
            .iter()
            .position(|b| b.outputs.first() == Some(&binding) && b.runtime.custom().is_some())
            .ok_or_else(|| {
                ChartError::new(ErrorCode::InvalidHandle, "unknown custom study binding")
            })?;
        let usable = |id| {
            self.series_entry(id).is_some()
                && !self
                    .indicators
                    .iter()
                    .skip(index)
                    .any(|b| b.outputs.contains(&id))
        };
        let type_id = match &self.indicators[index].kind {
            IndicatorKind::Custom { type_id, .. } => type_id,
            _ => unreachable!(),
        };
        if !usable(source)
            || volume.is_some_and(|id| {
                id == source
                    || !usable(id)
                    || self
                        .series_entry(id)
                        .is_none_or(|s| !s.kind.stores_scalar_values())
            })
            || (volume.is_some()
                && self
                    .custom_studies
                    .get(type_id)
                    .is_some_and(|d| !d.definition.uses_volume))
        {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "invalid custom study dependency",
            ));
        }
        self.indicators[index].source = source;
        self.indicators[index].source_input = source_input;
        self.indicators[index].volume_source = volume;
        self.retry_custom_study(binding)
    }

    pub fn custom_marker_plot(&self, output: SeriesId) -> bool {
        self.indicators
            .iter()
            .find_map(|b| {
                let index = b.outputs.iter().position(|id| *id == output)?;
                let IndicatorKind::Custom {
                    type_id, version, ..
                } = &b.kind
                else {
                    return None;
                };
                Some(
                    self.custom_studies
                        .get(type_id)
                        .filter(|d| d.definition.version == *version)
                        .and_then(|d| d.definition.outputs.get(index))
                        .is_some_and(|o| o.plot == CustomStudyPlot::Marker),
                )
            })
            .unwrap_or(false)
    }

    /// Rebuild a custom study binding. Like the built-in runtimes it reads the binding-owned
    /// input columns (the retained aggregate price, the timestamp-aligned volume), stops at the
    /// source's last real row so a tick filling a pre-installed session slot stays a tail
    /// update, and writes through the shared output store; rows before an output's first value
    /// are trimmed, so warm-up and blank (pending or faulted) outputs carry no whitespace rows.
    pub(crate) fn rebuild_custom_indicator(
        &mut self,
        index: usize,
        from: usize,
        full_replace: bool,
        outputs: [Option<SeriesId>; aeris_charts_indicators::MAX_OUTPUTS],
    ) -> [Option<(SeriesId, IndicatorChange)>; aeris_charts_indicators::MAX_OUTPUTS] {
        let mut changes = [None; aeris_charts_indicators::MAX_OUTPUTS];
        let binding = &self.indicators[index];
        let IndicatorKind::Custom {
            type_id,
            version,
            parameters,
            ..
        } = &binding.kind
        else {
            unreachable!()
        };
        let def = self
            .custom_studies
            .get(type_id)
            .filter(|d| d.definition.version == *version);
        let parameters = parameters.clone();
        let source = binding.source;
        let volume_source = binding.volume_source;
        let source_input = binding.source_input;
        let count = binding.outputs.len();
        let source_generation = self.data.series_generation(source).unwrap_or(0);
        let volume_generation = volume_source.and_then(|id| self.data.series_generation(id));
        let Some((times, values)) = self.data.series_data(source) else {
            return changes;
        };
        let rows = times.len();
        let end = self.indicator_data_end(index, rows, full_replace);
        let times = &times[..end];
        let values = values.map(|column| &column[..end.min(column.len())]);
        let n = end;
        let volume_column = volume_source
            .and_then(|id| self.data.series_data(id))
            .map(|(column_times, values)| (column_times, values[3]));
        let binding = &mut self.indicators[index];
        let custom = binding.runtime.custom_mut().unwrap();
        let was_faulted = matches!(custom.state, CustomBindingState::Faulted(_));
        if matches!(&custom.state, CustomBindingState::Active { covered, .. }
            if (*covered > 0 && full_replace) || *covered > n)
        {
            custom.state = CustomBindingState::Pending;
        }
        if matches!(custom.state, CustomBindingState::Pending)
            && let Some(def) = def
            && let Ok(normalized) = normalize_params(&def.definition, &parameters)
            && normalized == parameters
            && def.definition.outputs.len() == count
        {
            match (def.factory)(&parameters) {
                Ok(runtime) => {
                    custom.state = CustomBindingState::Active {
                        runtime,
                        covered: 0,
                    }
                }
                Err(fault) => custom.state = CustomBindingState::Faulted(fault.message),
            }
        }
        let mut start = if full_replace { 0 } else { from.min(n) };
        if let CustomBindingState::Active { covered, .. } = &custom.state {
            // A tick past rows the runtime never saw (whitespace session slots it skipped)
            // resumes where the runtime stopped instead of rebuilding the history.
            if full_replace || *covered > n {
                start = 0;
            } else {
                start = start.min(*covered);
            }
        }
        // Inputs derive only rows `start..`: the aggregate price and the timestamp-aligned volume
        // are retained and extended, so the runtime gets full-length columns without a copy.
        let (close, price_rows) =
            price_input(&mut binding.inputs.price, source_input, values, start);
        let (volume, volume_rows) =
            binding
                .inputs
                .volume
                .full_column(times, volume_column, start, f64::NAN);
        let custom = binding.runtime.custom_mut().unwrap();
        let mut result = (0..count).map(|_| Vec::new()).collect::<Vec<_>>();
        let mut computed_rows = 0;
        if let CustomBindingState::Active { runtime, covered } = &mut custom.state {
            let tail = (start == *covered && start > 0) || (start + 1 == *covered && n == *covered);
            computed_rows = n - start;
            custom.stats.calls += 1;
            custom.stats.rows += computed_rows as u64;
            match runtime.compute(
                CustomStudyInput {
                    times,
                    open: values[0],
                    high: values[1],
                    low: values[2],
                    close,
                    volume,
                    from: start,
                    tail,
                },
                &mut result,
            ) {
                Ok(())
                    if result.iter().all(|v| {
                        v.len() == n - start
                            && v.iter()
                                .all(|x| x.is_nan() || (x.is_finite() && x.abs() <= MAX_SAFE_VALUE))
                    }) =>
                {
                    *covered = n;
                }
                Ok(()) => {
                    custom.state = CustomBindingState::Faulted("invalid custom study output".into())
                }
                Err(fault) => custom.state = CustomBindingState::Faulted(fault.message),
            }
        }
        let fault = if let CustomBindingState::Faulted(message) = &custom.state {
            let mut message = message.clone();
            if message.len() > 256 {
                let mut end = 256;
                while !message.is_char_boundary(end) {
                    end -= 1;
                }
                message.truncate(end);
            }
            custom.state = CustomBindingState::Faulted(message.clone());
            Some(message)
        } else {
            None
        };
        if fault.is_some() || matches!(custom.state, CustomBindingState::Pending) {
            // A new fault invalidates previously computed values; an already blank
            // binding only needs the source's changed suffix.
            if fault.is_some() && !was_faulted {
                start = 0;
            }
            result = (0..count).map(|_| vec![f64::NAN; n - start]).collect();
        }
        binding.inputs.work_rows = price_rows + volume_rows + computed_rows;
        binding.source_generation = source_generation;
        binding.volume_generation = volume_generation;
        binding.data_end = end;
        let newly_faulted = fault.is_some() && !was_faulted;
        if newly_faulted && let Some(message) = fault {
            if self.custom_study_faults.len() == MAX_CUSTOM_STUDY_FAULTS {
                self.custom_study_faults.pop_front();
            }
            self.custom_study_faults.push_back(CustomStudyFaultEvent {
                binding: outputs[0].unwrap(),
                message,
            });
        }
        let Some((times, _)) = self.data.series_data(source) else {
            return changes;
        };
        let existing_starts: [usize; aeris_charts_indicators::MAX_OUTPUTS] =
            std::array::from_fn(|slot| {
                outputs[slot]
                    .and_then(|output| self.data.series_data(output))
                    .and_then(|(output_times, _)| output_times.first().copied())
                    .and_then(|first| times[..n].binary_search(&first).ok())
                    .unwrap_or(n)
            });
        for (slot, output) in outputs.iter().flatten().copied().enumerate() {
            let rewrite = if start <= existing_starts[slot] {
                // Aligned built-in outputs have no rows before their first value. Keep
                // custom warm-up and wholly blank pending/faulted outputs identical:
                // NaN is whitespace, not an input price for a dependent study.
                let first_value = result[slot]
                    .iter()
                    .position(|value| !value.is_nan())
                    .unwrap_or(result[slot].len());
                let mut values = std::mem::take(&mut result[slot]);
                values.drain(..first_value);
                (start + first_value, OutputRows::Replace(values))
            } else {
                (start, OutputRows::Update(&result[slot]))
            };
            let (_, change) = store_indicator_output(
                &mut self.data,
                source,
                output,
                rewrite.0,
                rewrite.1,
                end,
                rows,
            );
            changes[slot] = change.map(|change| (output, change));
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn definition(id: &str) -> CustomStudyDefinition {
        CustomStudyDefinition {
            type_id: id.into(),
            version: 1,
            title: "Custom".into(),
            parameters: vec![IndicatorParameterDescriptor {
                name: "period".into(),
                parameter_type: IndicatorParameterType::Integer,
                default: serde_json::json!(2),
                min: Some(1.0),
                max: Some(10.0),
                options: None,
            }],
            outputs: vec![CustomStudyOutput {
                name: "Average".into(),
                plot: CustomStudyPlot::Line,
                pane: CustomStudyPane::Price,
                default_style: IndicatorOutputStyle {
                    visible: true,
                    ..Default::default()
                },
            }],
            uses_volume: false,
        }
    }

    struct Average {
        period: usize,
        calls: Arc<Mutex<Vec<(usize, bool)>>>,
    }

    impl CustomStudyRuntime for Average {
        fn compute(
            &mut self,
            input: CustomStudyInput<'_>,
            out: &mut [Vec<f64>],
        ) -> Result<(), CustomStudyFault> {
            self.calls.lock().unwrap().push((input.from, input.tail));
            for row in input.from..input.times.len() {
                out[0].push(if row + 1 < self.period {
                    f64::NAN
                } else {
                    input.close[row + 1 - self.period..=row].iter().sum::<f64>()
                        / self.period as f64
                });
            }
            Ok(())
        }
    }

    fn factory(calls: Arc<Mutex<Vec<(usize, bool)>>>) -> CustomStudyFactory {
        Box::new(move |params| {
            Ok(Box::new(Average {
                period: params["period"].as_u64().unwrap() as usize,
                calls: Arc::clone(&calls),
            }))
        })
    }

    fn assert_same_scalar(chart: &ChartEngine, actual: SeriesId, expected: SeriesId) {
        let (actual_times, actual_values) = chart.data.series_data(actual).unwrap();
        let (expected_times, expected_values) = chart.data.series_data(expected).unwrap();
        assert_eq!(actual_times, expected_times);
        assert_eq!(actual_values[3].len(), expected_values[3].len());
        for (actual, expected) in actual_values[3].iter().zip(expected_values[3]) {
            assert!(
                actual == expected || (actual.is_nan() && expected.is_nan()),
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn chained_studies_on_custom_sma_match_builtin_after_repairs() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        chart
            .register_custom_study(
                definition("warmup"),
                Box::new(|_| {
                    Ok(Box::new(Average {
                        period: 3,
                        calls: Arc::new(Mutex::new(Vec::new())),
                    }))
                }),
            )
            .unwrap();
        let times = (1..=14).map(|i| i as f64 * 60.0).collect::<Vec<_>>();
        let values = [
            10., 13., 12., 15., 17., 16., 19., 21., 18., 20., 22., 19., 23., 24.,
        ];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        let custom = chart
            .add_custom_study(
                "warmup",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let built = chart.add_sma(0, 3).unwrap();
        let paired = [
            (
                chart.add_rsi(custom, 5).unwrap(),
                chart.add_rsi(built, 5).unwrap(),
            ),
            (
                chart.add_ema(custom, 4).unwrap(),
                chart.add_ema(built, 4).unwrap(),
            ),
            (
                chart.add_bollinger(custom, 4, 2.0)[0],
                chart.add_bollinger(built, 4, 2.0)[0],
            ),
        ];
        let verify = |chart: &ChartEngine| {
            assert_same_scalar(chart, custom, built);
            for (actual, expected) in paired {
                assert_same_scalar(chart, actual, expected);
            }
        };
        verify(&chart);
        assert!(chart.update_series_bar(0, 900.0, [25.0; 4]));
        verify(&chart);
        assert!(chart.update_series_bar(0, 900.0, [26.0; 4]));
        verify(&chart);
        assert!(chart.update_series_bar(0, 480.0, [27.0; 4]));
        verify(&chart);
    }

    #[test]
    fn faulted_and_pending_custom_sources_have_no_rsi_values() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let times = (1..=12).map(|i| i as f64 * 60.0).collect::<Vec<_>>();
        let values = times.clone();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart
            .register_custom_study(definition("fault"), Box::new(|_| Ok(Box::new(Bad))))
            .unwrap();
        let fault = chart
            .add_custom_study(
                "fault",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let pending = chart
            .restore_custom_study(
                0,
                IndicatorInputSource::Close,
                IndicatorKind::Custom {
                    type_id: "pending.blank".into(),
                    version: 1,
                    parameters: BTreeMap::new(),
                    output_count: 1,
                },
                None,
            )
            .unwrap()[0];
        for source in [fault, pending] {
            assert!(chart.data.series_data(source).unwrap().0.is_empty());
            let rsi = chart.add_rsi(source, 3).unwrap();
            assert!(chart.data.series_data(rsi).unwrap().0.is_empty());
            assert!(chart.update_series_bar(0, 780.0, [780.0; 4]));
            assert!(chart.data.series_data(rsi).unwrap().0.is_empty());
        }
    }

    #[test]
    fn multi_output_custom_study_preserves_independent_warmup_and_interior_whitespace() {
        struct TwoOutputs;
        impl CustomStudyRuntime for TwoOutputs {
            fn compute(
                &mut self,
                input: CustomStudyInput<'_>,
                out: &mut [Vec<f64>],
            ) -> Result<(), CustomStudyFault> {
                for row in input.from..input.times.len() {
                    out[0].push(if row < 1 {
                        f64::NAN
                    } else {
                        (input.close[row - 1] + input.close[row]) / 2.0
                    });
                    out[1].push(if row < 3 || row == 7 {
                        f64::NAN
                    } else {
                        input.close[row]
                    });
                }
                Ok(())
            }
        }
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let mut def = definition("two.outputs");
        def.outputs.push(def.outputs[0].clone());
        chart
            .register_custom_study(def, Box::new(|_| Ok(Box::new(TwoOutputs))))
            .unwrap();
        let times = (1..=13).map(|i| i as f64 * 60.0).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &times, &times, &times, &times)
            .unwrap();
        let outputs = chart
            .add_custom_study(
                "two.outputs",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap();
        let reference = chart.add_sma(0, 2).unwrap();
        let paired = (
            chart.add_ema(outputs[0], 3).unwrap(),
            chart.add_ema(reference, 3).unwrap(),
        );
        let band = chart.add_bollinger(outputs[1], 3, 2.0)[0];
        let verify = |chart: &ChartEngine| {
            assert_same_scalar(chart, outputs[0], reference);
            assert_same_scalar(chart, paired.0, paired.1);
            let (times, values) = chart.data.series_data(outputs[1]).unwrap();
            assert_eq!(times[0], 240);
            assert!(values[3][4].is_nan()); // row 7 is an interior whitespace row
            let (band_times, band_values) = chart.data.series_data(band).unwrap();
            assert!(
                band_values[3][band_times.iter().position(|&time| time == 480).unwrap()].is_nan()
            );
        };
        verify(&chart);
        assert!(chart.update_series_bar(0, 840.0, [840.0; 4]));
        verify(&chart);
        assert!(chart.update_series_bar(0, 480.0, [480.0; 4]));
        verify(&chart);
    }

    #[test]
    fn invalid_definition_is_rejected_before_registration() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let definition = CustomStudyDefinition {
            type_id: "Bad id".into(),
            version: 1,
            title: "Bad".into(),
            parameters: vec![],
            outputs: vec![],
            uses_volume: false,
        };
        assert_eq!(
            chart
                .register_custom_study(definition, Box::new(|_| unreachable!()))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidOptions
        );
        assert!(chart.custom_studies.is_empty());
    }

    #[test]
    fn custom_sma_schedules_repairs_and_chains() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        chart
            .register_custom_study(definition("average"), factory(Arc::clone(&calls)))
            .unwrap();
        let values = [10.0, 12.0, 14.0, 16.0];
        chart
            .set_series_data(0, &[1.0, 2.0, 3.0, 4.0], &values, &values, &values, &values)
            .unwrap();
        let custom = chart
            .add_custom_study(
                "average",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let built_in = chart.add_sma(0, 2).unwrap();
        let chained = chart.add_sma(custom, 2).unwrap();
        let (custom_times, custom_values) = chart.data.series_data(custom).unwrap();
        let (built_times, built_values) = chart.data.series_data(built_in).unwrap();
        assert_eq!(custom_times, built_times);
        assert_eq!(custom_values[3], built_values[3]);
        assert_eq!(
            chart
                .indicator_info(custom)
                .unwrap()
                .parameters
                .custom
                .unwrap()["period"],
            2
        );
        assert_eq!(chart.indicator_info(custom).unwrap().output_name, "Average");
        assert!(chart.data.series_data(chained).is_some());
        assert!(chart.update_series_bar(0, 5.0, [18.0; 4]));
        assert!(chart.update_series_bar(0, 5.0, [20.0; 4]));
        let (custom_times, custom_values) = chart.data.series_data(custom).unwrap();
        let (built_times, built_values) = chart.data.series_data(built_in).unwrap();
        assert_eq!(custom_times, built_times);
        assert_eq!(custom_values[3], built_values[3]);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[(0, false), (4, true), (4, true)]
        );
        assert_eq!(chart.custom_study_stats(custom).unwrap().calls, 3);
        assert!(chart.update_series_bar(0, 3.0, [30.0; 4]));
        assert_eq!(calls.lock().unwrap().last(), Some(&(2, false)));
        let (custom_times, custom_values) = chart.data.series_data(custom).unwrap();
        let (built_times, built_values) = chart.data.series_data(built_in).unwrap();
        assert_eq!(custom_times, built_times);
        assert_eq!(custom_values[3], built_values[3]);
        chart
            .set_series_data(
                0,
                &[10.0, 11.0],
                &[3.0, 5.0],
                &[3.0, 5.0],
                &[3.0, 5.0],
                &[3.0, 5.0],
            )
            .unwrap();
        assert_eq!(calls.lock().unwrap().last(), Some(&(0, false)));
        let (_, values) = chart.data.series_data(custom).unwrap();
        assert_eq!(values[3], [4.0]);
        assert!(chart.data.series_data(chained).is_some());
    }

    #[test]
    fn pending_binding_activates_and_invalid_parameters_do_not_mutate() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let values = [1.0, 2.0, 3.0];
        chart
            .set_series_data(0, &[1.0, 2.0, 3.0], &values, &values, &values, &values)
            .unwrap();
        let kind = IndicatorKind::Custom {
            type_id: "late".into(),
            version: 1,
            parameters: BTreeMap::from([("period".into(), serde_json::json!(2))]),
            output_count: 1,
        };
        let output = chart
            .restore_custom_study(0, IndicatorInputSource::Close, kind.clone(), None)
            .unwrap()[0];
        assert!(!chart.custom_study_is_resolved(output));
        assert!(
            chart.data.series_data(output).unwrap().1[3]
                .iter()
                .all(|x| x.is_nan())
        );
        assert_eq!(chart.indicator_bindings()[0].kind, kind);
        chart
            .register_custom_study(
                definition("late"),
                factory(Arc::new(Mutex::new(Vec::new()))),
            )
            .unwrap();
        assert!(chart.custom_study_is_resolved(output));
        assert_eq!(chart.data.series_data(output).unwrap().1[3], [1.5, 2.5]);
        let before = chart.indicator_bindings().len();
        assert_eq!(
            chart
                .add_custom_study(
                    "late",
                    0,
                    IndicatorInputSource::Close,
                    None,
                    BTreeMap::from([("period".into(), serde_json::json!(11))])
                )
                .unwrap_err()
                .code(),
            ErrorCode::InvalidOptions
        );
        assert_eq!(chart.indicator_bindings().len(), before);
    }

    #[test]
    fn pending_import_preserves_dedicated_pane_order_and_activation_presentation() {
        let values = [1.0, 2.0, 3.0, 4.0];
        let mut original = ChartEngine::new(800.0, 600.0, 1.0);
        original
            .set_series_data(0, &[1.0, 2.0, 3.0, 4.0], &values, &values, &values, &values)
            .unwrap();
        let mut def = definition("late.pane");
        def.title = "Late Study".into();
        def.outputs[0].name = "Signal".into();
        def.outputs[0].pane = CustomStudyPane::Dedicated;
        def.outputs[0].plot = CustomStudyPlot::Histogram;
        original
            .register_custom_study(def.clone(), factory(Arc::new(Mutex::new(Vec::new()))))
            .unwrap();
        let first = original
            .add_custom_study(
                "late.pane",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let style = IndicatorOutputStyle {
            visible: true,
            line_color: Some("#abcdef".into()),
            ..Default::default()
        };
        assert!(original.set_indicator_output_style(first, style.clone()));
        let later = original.add_rsi(0, 2).unwrap();
        let expected_panes = [
            original.series_entry(first).unwrap().pane_index,
            original.series_entry(later).unwrap().pane_index,
        ];
        let document = original.export_state_json().unwrap();

        let mut restored = ChartEngine::new(800.0, 600.0, 1.0);
        restored
            .set_series_data(0, &[1.0, 2.0, 3.0, 4.0], &values, &values, &values, &values)
            .unwrap();
        let result = restored.import_state_json(&document).unwrap();
        assert_eq!(result.unresolved_custom_studies.len(), 1);
        let pending = restored.indicator_bindings()[0].outputs[0];
        let later = restored.indicator_bindings()[1].outputs[0];
        assert_eq!(
            [
                restored.series_entry(pending).unwrap().pane_index,
                restored.series_entry(later).unwrap().pane_index,
            ],
            expected_panes
        );
        assert_eq!(restored.export_state_json().unwrap(), document);
        restored
            .register_custom_study(def, factory(Arc::new(Mutex::new(Vec::new()))))
            .unwrap();
        assert_eq!(
            restored.series_entry(pending).unwrap().pane_index,
            expected_panes[0]
        );
        assert_eq!(
            restored.series_entry(pending).unwrap().title,
            "Late Study Signal"
        );
        assert_eq!(
            restored
                .comparison_legend_snapshot()
                .iter()
                .find(|entry| entry.series_id == pending)
                .unwrap()
                .title,
            "Late Study Signal"
        );
        assert_eq!(
            restored.series_entry(pending).unwrap().kind,
            SeriesKind::Histogram
        );
        assert_eq!(restored.indicator_info(pending).unwrap().style, style);
        assert_eq!(
            restored.series_entry(later).unwrap().pane_index,
            expected_panes[1]
        );
    }

    #[test]
    fn pending_and_faulted_tail_updates_leave_chained_sma_work_bounded() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let n = 10_000;
        let times = (1..=n).map(|row| row as f64).collect::<Vec<_>>();
        let values = times.clone();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        let kind = IndicatorKind::Custom {
            type_id: "missing".into(),
            version: 1,
            parameters: BTreeMap::new(),
            output_count: 1,
        };
        let pending = chart
            .restore_custom_study(0, IndicatorInputSource::Close, kind, None)
            .unwrap()[0];
        let chained = chart.add_sma(pending, 2).unwrap();
        assert!(chart.update_series_bar(0, (n + 1) as f64, [(n + 1) as f64; 4]));
        assert!(chart.last_indicator_work_rows() < 100);
        assert_eq!(chart.indicator_changes[0].1.from, n);
        assert!(
            chart.data.series_data(chained).unwrap().1[3]
                .iter()
                .all(|v| v.is_nan())
        );

        chart
            .register_custom_study(definition("bad"), Box::new(|_| Ok(Box::new(Bad))))
            .unwrap();
        let faulted = chart
            .add_custom_study("bad", 0, IndicatorInputSource::Close, None, BTreeMap::new())
            .unwrap()[0];
        let chained_fault = chart.add_sma(faulted, 2).unwrap();
        assert!(chart.update_series_bar(0, (n + 2) as f64, [(n + 2) as f64; 4]));
        assert!(chart.last_indicator_work_rows() < 100);
        assert_eq!(chart.indicator_changes[0].1.from, n + 1);
        assert!(
            chart.data.series_data(chained_fault).unwrap().1[3]
                .iter()
                .all(|v| v.is_nan())
        );
    }

    struct Bad;
    impl CustomStudyRuntime for Bad {
        fn compute(
            &mut self,
            _: CustomStudyInput<'_>,
            out: &mut [Vec<f64>],
        ) -> Result<(), CustomStudyFault> {
            out[0].push(f64::INFINITY);
            Ok(())
        }
    }

    #[test]
    fn invalid_output_faults_once_and_retry_requeues() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        chart
            .register_custom_study(definition("bad"), Box::new(|_| Ok(Box::new(Bad))))
            .unwrap();
        let values = [1.0, 2.0];
        chart
            .set_series_data(0, &[1.0, 2.0], &values, &values, &values, &values)
            .unwrap();
        let output = chart
            .add_custom_study("bad", 0, IndicatorInputSource::Close, None, BTreeMap::new())
            .unwrap()[0];
        assert_eq!(chart.take_custom_study_faults().len(), 1);
        assert!(
            chart.data.series_data(output).unwrap().1[3]
                .iter()
                .all(|x| x.is_nan())
        );
        assert!(chart.update_series_bar(0, 3.0, [3.0; 4]));
        assert_eq!(chart.custom_study_stats(output).unwrap().calls, 1);
        assert!(chart.take_custom_study_faults().is_empty());
        chart.retry_custom_study(output).unwrap();
        assert_eq!(chart.custom_study_stats(output).unwrap().calls, 2);
        assert_eq!(chart.take_custom_study_faults().len(), 1);
    }

    #[test]
    fn registration_binding_and_fault_queues_have_hard_caps() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        for i in 0..MAX_CUSTOM_STUDY_TYPES {
            chart
                .register_custom_study(
                    definition(&format!("study.{i}")),
                    factory(Arc::new(Mutex::new(Vec::new()))),
                )
                .unwrap();
        }
        assert_eq!(
            chart
                .register_custom_study(
                    definition("overflow"),
                    factory(Arc::new(Mutex::new(Vec::new())))
                )
                .unwrap_err()
                .code(),
            ErrorCode::ResourceLimit
        );
        let mut outputs = Vec::new();
        for _ in 0..MAX_CUSTOM_STUDY_BINDINGS {
            outputs.push(
                chart
                    .add_custom_study(
                        "study.0",
                        0,
                        IndicatorInputSource::Close,
                        None,
                        BTreeMap::new(),
                    )
                    .unwrap()[0],
            );
        }
        assert_eq!(
            chart
                .add_custom_study(
                    "study.0",
                    0,
                    IndicatorInputSource::Close,
                    None,
                    BTreeMap::new()
                )
                .unwrap_err()
                .code(),
            ErrorCode::ResourceLimit
        );
        assert!(chart.remove_indicator_binding(outputs[0]));
        assert!(
            chart
                .add_custom_study(
                    "study.0",
                    0,
                    IndicatorInputSource::Close,
                    None,
                    BTreeMap::new()
                )
                .is_ok()
        );
        for i in 0..MAX_CUSTOM_STUDY_FAULTS + 5 {
            chart.custom_study_faults.push_back(CustomStudyFaultEvent {
                binding: i as SeriesId,
                message: "fault".into(),
            });
            if chart.custom_study_faults.len() > MAX_CUSTOM_STUDY_FAULTS {
                chart.custom_study_faults.pop_front();
            }
        }
        let events = chart.take_custom_study_faults();
        assert_eq!(events.len(), MAX_CUSTOM_STUDY_FAULTS);
        assert_eq!(events[0].binding, 5);
        assert!(chart.take_custom_study_faults().is_empty());
    }

    #[test]
    fn active_persistence_and_version_mismatch_preserve_custom_definition() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut original = ChartEngine::new(800.0, 600.0, 1.0);
        original
            .register_custom_study(definition("persisted"), factory(Arc::clone(&calls)))
            .unwrap();
        let values = [4.0, 6.0, 8.0, 10.0];
        original
            .set_series_data(0, &[1.0, 2.0, 3.0, 4.0], &values, &values, &values, &values)
            .unwrap();
        let output = original
            .add_custom_study(
                "persisted",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let style = IndicatorOutputStyle {
            visible: true,
            line_color: Some("#ff0000".into()),
            ..Default::default()
        };
        assert!(original.set_indicator_output_style(output, style.clone()));
        let document = original.export_state_json().unwrap();

        let restore = |version: u32| {
            let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
            let mut def = definition("persisted");
            def.version = version;
            chart
                .register_custom_study(def, factory(Arc::new(Mutex::new(Vec::new()))))
                .unwrap();
            chart
                .set_series_data(0, &[1.0, 2.0, 3.0, 4.0], &values, &values, &values, &values)
                .unwrap();
            let result = chart.import_state_json(&document).unwrap();
            (chart, result)
        };
        let (active, result) = restore(1);
        assert!(result.unresolved_custom_studies.is_empty());
        let binding = active.indicator_bindings()[0].clone();
        assert_eq!(binding.kind, original.indicator_bindings()[0].kind);
        assert_eq!(binding.styles, vec![style.clone()]);
        assert_eq!(
            active.data.series_data(binding.outputs[0]).unwrap().1[3],
            original.data.series_data(output).unwrap().1[3]
        );
        let (pending, result) = restore(2);
        assert_eq!(result.unresolved_custom_studies.len(), 1);
        assert!(
            pending
                .data
                .series_data(pending.indicator_bindings()[0].outputs[0])
                .unwrap()
                .1[3]
                .iter()
                .all(|v| v.is_nan())
        );
        let pending_json: serde_json::Value =
            serde_json::from_str(&pending.export_state_json().unwrap()).unwrap();
        let original_json: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(
            pending_json["indicators"][0]["kind"],
            original_json["indicators"][0]["kind"]
        );
        let mut invalid_document = original_json.clone();
        invalid_document["indicators"][0]["kind"]["parameters"]["period"] = serde_json::json!(999);
        let mut rejected = ChartEngine::new(800.0, 600.0, 1.0);
        rejected
            .register_custom_study(
                definition("persisted"),
                factory(Arc::new(Mutex::new(Vec::new()))),
            )
            .unwrap();
        let rejected_before = rejected.series.len();
        assert!(
            rejected
                .import_state_json(&invalid_document.to_string())
                .is_err()
        );
        assert_eq!(rejected.series.len(), rejected_before);
        assert!(rejected.indicator_bindings().is_empty());
    }

    #[test]
    fn fault_clears_dependent_and_retry_recovers_without_stale_values() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Toggle {
            broken: Arc<AtomicBool>,
        }
        impl CustomStudyRuntime for Toggle {
            fn compute(
                &mut self,
                input: CustomStudyInput<'_>,
                out: &mut [Vec<f64>],
            ) -> Result<(), CustomStudyFault> {
                if self.broken.load(Ordering::Relaxed) {
                    return Err(CustomStudyFault::from("upstream failed"));
                }
                out[0].extend_from_slice(&input.close[input.from..]);
                Ok(())
            }
        }
        let broken = Arc::new(AtomicBool::new(false));
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let state = Arc::clone(&broken);
        chart
            .register_custom_study(
                definition("toggle"),
                Box::new(move |_| {
                    Ok(Box::new(Toggle {
                        broken: Arc::clone(&state),
                    }))
                }),
            )
            .unwrap();
        chart
            .set_series_data(
                0,
                &[1.0, 2.0],
                &[1.0, 2.0],
                &[1.0, 2.0],
                &[1.0, 2.0],
                &[1.0, 2.0],
            )
            .unwrap();
        let output = chart
            .add_custom_study(
                "toggle",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let dependent = chart.add_sma(output, 2).unwrap();
        assert_eq!(chart.data.series_data(dependent).unwrap().1[3], [1.5]);
        broken.store(true, Ordering::Relaxed);
        assert!(chart.update_series_bar(0, 3.0, [3.0; 4]));
        assert_eq!(chart.take_custom_study_faults().len(), 1);
        assert!(
            chart.data.series_data(output).unwrap().1[3]
                .iter()
                .all(|v| v.is_nan())
        );
        assert!(
            chart.data.series_data(dependent).unwrap().1[3]
                .iter()
                .all(|v| v.is_nan())
        );
        assert!(chart.update_series_bar(0, 4.0, [4.0; 4]));
        assert_eq!(chart.custom_study_stats(output).unwrap().calls, 2);
        broken.store(false, Ordering::Relaxed);
        chart.retry_custom_study(output).unwrap();
        assert_eq!(
            chart.data.series_data(output).unwrap().1[3],
            [1.0, 2.0, 3.0, 4.0]
        );
        assert_eq!(
            chart.data.series_data(dependent).unwrap().1[3],
            [1.5, 2.5, 3.5]
        );
    }

    /// One recorded compute call: `from`, `tail`, rows handed over, the close column's address,
    /// and its last value.
    type ProbeCall = (usize, bool, usize, usize, f64);

    /// A two-row average that records how each call reads its input.
    struct Probe {
        calls: Arc<Mutex<Vec<ProbeCall>>>,
    }

    impl CustomStudyRuntime for Probe {
        fn compute(
            &mut self,
            input: CustomStudyInput<'_>,
            out: &mut [Vec<f64>],
        ) -> Result<(), CustomStudyFault> {
            self.calls.lock().unwrap().push((
                input.from,
                input.tail,
                input.times.len(),
                input.close.as_ptr() as usize,
                input.close.last().copied().unwrap_or(f64::NAN),
            ));
            for row in input.from..input.times.len() {
                out[0].push(if row == 0 {
                    f64::NAN
                } else {
                    (input.close[row - 1] + input.close[row]) * 0.5
                });
            }
            Ok(())
        }
    }

    fn probe(calls: &Arc<Mutex<Vec<ProbeCall>>>) -> CustomStudyFactory {
        let calls = Arc::clone(calls);
        Box::new(move |_| {
            Ok(Box::new(Probe {
                calls: Arc::clone(&calls),
            }))
        })
    }

    fn custom_binding(chart: &ChartEngine, output: SeriesId) -> &IndicatorBinding {
        chart
            .indicators
            .iter()
            .find(|binding| binding.outputs[0] == output)
            .unwrap()
    }

    #[test]
    fn aggregate_input_custom_study_reuses_its_retained_price_column_per_tick() {
        // Fork guarantee (X8): an hl2 source reads the binding-owned aggregate column, extended
        // by the changed rows only, never a per-tick O(n) copy of the source.
        const ROWS: usize = 10_000;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        chart
            .register_custom_study(definition("probe"), probe(&calls))
            .unwrap();
        let times = (0..ROWS).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
        let low = (0..ROWS).map(|row| row as f64).collect::<Vec<_>>();
        let high = low.iter().map(|value| value + 2.0).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &low, &high, &low, &low)
            .unwrap();
        let output = chart
            .add_custom_study("probe", 0, IndicatorInputSource::Hl2, None, BTreeMap::new())
            .unwrap()[0];
        let cache = custom_binding(&chart, output).inputs.price.as_ptr() as usize;
        assert_eq!(calls.lock().unwrap().last().unwrap().3, cache);
        for (tick, row) in [ROWS, ROWS, ROWS + 1, ROWS + 2, ROWS + 2]
            .into_iter()
            .enumerate()
        {
            let low = row as f64 + tick as f64 * 0.25;
            assert!(chart.update_series_bar(0, row as f64 * 60.0, [low, low + 2.0, low, low]));
            let binding = custom_binding(&chart, output);
            let (from, tail, rows, close, last) = *calls.lock().unwrap().last().unwrap();
            assert!(tail, "tick {tick} must take the tail path");
            assert_eq!(rows, row + 1);
            assert!(
                from >= row.saturating_sub(1),
                "tick {tick} recomputed from {from}"
            );
            assert_eq!(close, binding.inputs.price.as_ptr() as usize, "tick {tick}");
            assert_eq!(close, cache, "tick {tick} reallocated the aggregate column");
            assert_eq!(last, low + 1.0);
            assert!(
                binding.last_work_rows() <= 6,
                "tick {tick} did {} work rows",
                binding.last_work_rows()
            );
        }
    }

    #[test]
    fn custom_volume_column_spans_every_source_row_while_volume_trails() {
        // Runtime contract: `volume` is empty or exactly `times.len()` long, so a runtime may index
        // it by source row even while the volume series trails the source by a bar.
        struct VolumeEcho;
        impl CustomStudyRuntime for VolumeEcho {
            fn compute(
                &mut self,
                input: CustomStudyInput<'_>,
                out: &mut [Vec<f64>],
            ) -> Result<(), CustomStudyFault> {
                assert_eq!(input.volume.len(), input.times.len());
                for row in input.from..input.times.len() {
                    out[0].push(input.volume[row]);
                }
                Ok(())
            }
        }
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let mut volume_definition = definition("volume");
        volume_definition.uses_volume = true;
        chart
            .register_custom_study(
                volume_definition,
                Box::new(|_| Ok(Box::new(VolumeEcho) as Box<dyn CustomStudyRuntime>)),
            )
            .unwrap();
        let times = (0..4).map(|row| row as f64 * 60.0).collect::<Vec<_>>();
        let prices = [1.0; 4];
        chart
            .set_series_data(0, &times, &prices, &prices, &prices, &prices)
            .unwrap();
        let volume = chart.add_series(SeriesKind::Histogram);
        let sizes = [10.0, 11.0, 12.0, 13.0];
        chart
            .set_series_data(volume, &times, &sizes, &sizes, &sizes, &sizes)
            .unwrap();
        let output = chart
            .add_custom_study(
                "volume",
                0,
                IndicatorInputSource::Close,
                Some(volume),
                BTreeMap::new(),
            )
            .unwrap()[0];
        let column = |chart: &ChartEngine| chart.data.series_data(output).unwrap().1[3].to_vec();
        // The candle streams its next bar before its volume does.
        assert!(chart.update_series_bar(0, 240.0, [1.0; 4]));
        let values = column(&chart);
        assert_eq!(values.len(), 5);
        assert_eq!(values[..4], sizes);
        assert!(values[4].is_nan());
        assert!(chart.update_series_bar(volume, 240.0, [14.0; 4]));
        assert_eq!(column(&chart), [10.0, 11.0, 12.0, 13.0, 14.0]);
    }

    #[test]
    fn custom_study_ticks_filling_pre_installed_session_slots_stay_tail_updates() {
        // Fork guarantee (X7): the custom runtime stops at the source's last real row, so a tick
        // filling a whitespace session slot is a tail update over the changed rows, not a
        // recomputation through every trailing slot.
        const ROWS: usize = 200;
        const SLOTS: usize = 100;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        chart
            .register_custom_study(definition("probe"), probe(&calls))
            .unwrap();
        let times = (0..ROWS + SLOTS)
            .map(|row| row as f64 * 60.0)
            .collect::<Vec<_>>();
        let values = (0..ROWS + SLOTS)
            .map(|row| if row < ROWS { row as f64 } else { f64::NAN })
            .collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        let output = chart
            .add_custom_study(
                "probe",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        assert_eq!(calls.lock().unwrap().last().unwrap().2, ROWS);
        let fill = |chart: &mut ChartEngine, row: usize, value: f64| {
            assert!(chart.update_series_bar(0, row as f64 * 60.0, [value; 4]));
        };
        for (row, value) in [(ROWS, 1.0), (ROWS, 2.0), (ROWS + 1, 3.0), (ROWS + 3, 4.0)] {
            fill(&mut chart, row, value);
            let (from, tail, rows, _, _) = *calls.lock().unwrap().last().unwrap();
            assert!(tail, "slot {row} must take the tail path");
            assert_eq!(rows, row + 1, "slot {row} must stop at the last real row");
            assert!(row + 1 - from <= 3, "slot {row} recomputed from {from}");
        }
        // The output keeps every slot row, and equals a fresh install over the same rows.
        let (output_times, output_values) = chart.data.series_data(output).unwrap();
        assert_eq!(
            *output_times.last().unwrap(),
            ((ROWS + SLOTS - 1) * 60) as i64
        );
        let actual = output_values[3].to_vec();
        let fresh_calls = Arc::new(Mutex::new(Vec::new()));
        let mut fresh = ChartEngine::new(800.0, 600.0, 1.0);
        fresh
            .register_custom_study(definition("probe"), probe(&fresh_calls))
            .unwrap();
        let (times, columns) = chart.data.series_data(0).unwrap();
        let times = times.iter().map(|&time| time as f64).collect::<Vec<_>>();
        let columns = columns.map(<[f64]>::to_vec);
        fresh
            .set_series_data(
                0,
                &times,
                &columns[0],
                &columns[1],
                &columns[2],
                &columns[3],
            )
            .unwrap();
        let fresh_output = fresh
            .add_custom_study(
                "probe",
                0,
                IndicatorInputSource::Close,
                None,
                BTreeMap::new(),
            )
            .unwrap()[0];
        let expected = fresh.data.series_data(fresh_output).unwrap().1[3].to_vec();
        assert_eq!(actual.len(), expected.len());
        for (row, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert!(
                actual == expected || (actual.is_nan() && expected.is_nan()),
                "row {row}: {actual} != {expected}"
            );
        }
    }
}
