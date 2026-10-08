//! Engine-owned projection of host-computed scalar studies.
//!
//! A host owns provider subscriptions and computes values. The chart engine owns validation,
//! generation fencing, series/pane lifecycle, presentation, and group operations.

use crate::{
    ChartEngine, EMA_RIBBON_DEFAULT_COLORS, IndicatorChromeOptions,
    SEPARATE_INDICATOR_PANE_STRETCH, SeriesId, SeriesKind, SeriesThresholdRegion,
};
use aeris_charts_core::model::data_validation::{MAX_SAFE_VALUE, validate_timestamp};
use std::fmt;

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const MAX_EXTERNAL_STUDY_TITLE_BYTES: usize = 256;
const MAX_EXTERNAL_STUDY_LEGEND_LABEL_BYTES: usize = 128;

/// Hard chart-local bound for host-computed scalar outputs.
pub const MAX_EXTERNAL_STUDY_OUTPUTS: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExternalStudyInputRequirements(u8);

impl ExternalStudyInputRequirements {
    const BARS_BIT: u8 = 1 << 0;
    const TRADES_BIT: u8 = 1 << 1;
    const QUOTES_BIT: u8 = 1 << 2;
    const DEPTH_BIT: u8 = 1 << 3;

    pub const NONE: Self = Self(0);
    pub const BARS: Self = Self(Self::BARS_BIT);

    #[must_use]
    pub const fn with(self, stream: ExternalStudyInputStream) -> Self {
        let bit = match stream {
            ExternalStudyInputStream::Bars => Self::BARS_BIT,
            ExternalStudyInputStream::Trades => Self::TRADES_BIT,
            ExternalStudyInputStream::Quotes => Self::QUOTES_BIT,
            ExternalStudyInputStream::Depth => Self::DEPTH_BIT,
        };
        Self(self.0 | bit)
    }

    #[must_use]
    pub const fn contains(self, stream: ExternalStudyInputStream) -> bool {
        let bit = match stream {
            ExternalStudyInputStream::Bars => Self::BARS_BIT,
            ExternalStudyInputStream::Trades => Self::TRADES_BIT,
            ExternalStudyInputStream::Quotes => Self::QUOTES_BIT,
            ExternalStudyInputStream::Depth => Self::DEPTH_BIT,
        };
        self.0 & bit != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStudyInputStream {
    Bars,
    Trades,
    Quotes,
    Depth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStudyPlotKind {
    Line,
    Histogram,
    Area,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStudyPaneTarget {
    Price,
    Dedicated { group: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStudyScaleTarget {
    Primary,
    Left,
    Overlay,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalStudyThresholdRegion {
    pub lower: f64,
    pub upper: f64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExternalStudyPointStyle {
    #[default]
    Uniform,
    MomentumHistogram,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalStudyOutputDescriptor<'a> {
    pub title: &'a str,
    pub legend_label: Option<&'a str>,
    pub plot: ExternalStudyPlotKind,
    pub pane: ExternalStudyPaneTarget,
    pub scale: ExternalStudyScaleTarget,
    pub settings_available: bool,
    pub threshold_region: Option<ExternalStudyThresholdRegion>,
    pub point_style: ExternalStudyPointStyle,
    pub input_requirements: ExternalStudyInputRequirements,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalStudyOutputInfo {
    pub study_id: u64,
    pub output_index: usize,
    pub series_id: SeriesId,
    pub generation: u64,
    pub settings_available: bool,
    pub legend_label: Option<String>,
    pub input_requirements: ExternalStudyInputRequirements,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExternalStudyOutputState {
    pub(crate) series_id: SeriesId,
    generation: u64,
    settings_available: bool,
    legend_label: Option<String>,
    input_requirements: ExternalStudyInputRequirements,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStudyError {
    LengthMismatch,
    UnsupportedTimestampPrecision,
    NonIncreasingTimestamp,
    InvalidValue,
    InvalidPresentation,
    MetadataTooLong,
    CapacityExceeded,
    InstallationRejected,
}

impl fmt::Display for ExternalStudyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LengthMismatch => "study timestamps and values must have the same length",
            Self::UnsupportedTimestampPrecision => {
                "study timestamps must be whole supported UTC seconds"
            }
            Self::NonIncreasingTimestamp => "study timestamps must be strictly increasing",
            Self::InvalidValue => {
                "study values must be finite and within the supported scalar range"
            }
            Self::InvalidPresentation => "study presentation is invalid for its plot kind",
            Self::MetadataTooLong => {
                "study title or legend label exceeds the bounded metadata limit"
            }
            Self::CapacityExceeded => "chart external-study output capacity is exhausted",
            Self::InstallationRejected => "chart rejected the study output",
        })
    }
}

impl std::error::Error for ExternalStudyError {}

struct PreparedColumns {
    times: Vec<f64>,
    scalar: Vec<f64>,
}

impl ChartEngine {
    pub fn install_external_study_output(
        &mut self,
        study_id: u64,
        output_index: usize,
        descriptor: ExternalStudyOutputDescriptor<'_>,
        generation: u64,
        timestamps_unix_nanos: &[i64],
        values: &[Option<f64>],
    ) -> Result<bool, ExternalStudyError> {
        let key = (study_id, output_index);
        if self
            .external_study_outputs
            .get(&key)
            .is_some_and(|state| state.generation >= generation)
        {
            return Ok(false);
        }
        validate_descriptor(descriptor)?;
        let columns = prepare_columns(timestamps_unix_nanos, values)?;
        let existing = self.external_study_outputs.get(&key).cloned();
        if existing.is_none() && self.external_study_outputs.len() >= MAX_EXTERNAL_STUDY_OUTPUTS {
            return Err(ExternalStudyError::CapacityExceeded);
        }
        let series_id = if let Some(state) = existing {
            set_series_data(self, state.series_id, &columns)?;
            state.series_id
        } else {
            self.install_new_external_study_series(study_id, output_index, descriptor, &columns)?
        };
        apply_presentation(self, series_id, output_index, descriptor);
        self.external_study_outputs.insert(
            key,
            ExternalStudyOutputState {
                series_id,
                generation,
                settings_available: descriptor.settings_available,
                legend_label: descriptor.legend_label.map(str::to_owned),
                input_requirements: descriptor.input_requirements,
            },
        );
        Ok(true)
    }

    fn install_new_external_study_series(
        &mut self,
        study_id: u64,
        output_index: usize,
        descriptor: ExternalStudyOutputDescriptor<'_>,
        columns: &PreparedColumns,
    ) -> Result<SeriesId, ExternalStudyError> {
        let source_price_format = self.series_entry(0).map(|series| {
            (
                series.price_format.kind,
                series.price_format.precision,
                series.price_format.min_move,
            )
        });
        let chrome = self.indicator_chrome;
        let series_id = self.add_series(series_kind(descriptor.plot));
        if let Err(error) = set_series_data(self, series_id, columns) {
            let _ = self.remove_series(series_id);
            return Err(error);
        }
        if let Some(series) = self.series_entry_mut(series_id) {
            series.title = descriptor.title.to_owned();
            series.title_visible = chrome.name_labels_visible;
            series.last_value_visible = chrome.value_labels_visible;
            series.price_line_visible = chrome.price_lines_visible;
            series.line_width = Some(2.0);
            if let Some((kind, precision, min_move)) = source_price_format {
                series.price_format.kind = kind;
                series.price_format.precision = precision;
                series.price_format.min_move = min_move;
            }
        }
        self.adopt_scale_price_format(series_id);
        let mut created_pane = None;
        let pane_index = match descriptor.pane {
            ExternalStudyPaneTarget::Price => 0,
            ExternalStudyPaneTarget::Dedicated { group } => {
                let pane_key = (study_id, group);
                if let Some(index) = self
                    .external_study_panes
                    .get(&pane_key)
                    .and_then(|pane_id| self.pane_index_for_id(*pane_id))
                {
                    index
                } else {
                    self.external_study_panes.remove(&pane_key);
                    let Some(index) = self.add_pane(false) else {
                        let _ = self.remove_series(series_id);
                        return Err(ExternalStudyError::InstallationRejected);
                    };
                    let Some(pane_id) = self.pane_stable_id(index) else {
                        let _ = self.remove_series(series_id);
                        let _ = self.remove_pane(index);
                        return Err(ExternalStudyError::InstallationRejected);
                    };
                    created_pane = Some((pane_key, pane_id, index));
                    index
                }
            }
        };
        if !self.try_set_series_pane_and_scale(
            series_id,
            pane_index,
            SEPARATE_INDICATOR_PANE_STRETCH,
            scale_id(descriptor.scale),
        ) {
            let _ = self.remove_series(series_id);
            if let Some((_, _, index)) = created_pane {
                let _ = self.remove_pane(index);
            }
            return Err(ExternalStudyError::InstallationRejected);
        }
        if let Some((key, pane_id, _)) = created_pane {
            self.external_study_panes.insert(key, pane_id);
        }
        apply_presentation(self, series_id, output_index, descriptor);
        Ok(series_id)
    }

    #[must_use]
    pub fn external_study_output_info(
        &self,
        study_id: u64,
        output_index: usize,
    ) -> Option<ExternalStudyOutputInfo> {
        let state = self.external_study_outputs.get(&(study_id, output_index))?;
        self.series_entry(state.series_id)?;
        Some(output_info(study_id, output_index, state))
    }

    #[must_use]
    pub fn external_study_outputs(&self) -> Vec<ExternalStudyOutputInfo> {
        self.external_study_outputs
            .iter()
            .filter_map(|(&(study_id, output_index), state)| {
                self.series_entry(state.series_id)
                    .map(|_| output_info(study_id, output_index, state))
            })
            .collect()
    }

    /// Whether the chart currently owns at least one live external study output.
    #[must_use]
    pub fn has_external_studies(&self) -> bool {
        self.external_study_outputs
            .values()
            .any(|state| self.series_entry(state.series_id).is_some())
    }

    /// Return the runtime study that owns a live output series.
    #[must_use]
    pub fn external_study_for_series(&self, series_id: SeriesId) -> Option<u64> {
        self.external_study_outputs
            .iter()
            .find_map(|(&(study_id, _), state)| {
                (state.series_id == series_id && self.series_entry(series_id).is_some())
                    .then_some(study_id)
            })
    }

    pub(crate) fn external_study_series(&self, study_id: u64) -> Vec<SeriesId> {
        self.external_study_outputs
            .iter()
            .filter_map(|(&(candidate, _), state)| {
                (candidate == study_id && self.series_entry(state.series_id).is_some())
                    .then_some(state.series_id)
            })
            .collect()
    }

    #[must_use]
    pub fn external_study_visible(&self, study_id: u64) -> Option<bool> {
        let mut found = false;
        let mut visible = true;
        for (&(candidate, _), state) in &self.external_study_outputs {
            if candidate != study_id {
                continue;
            }
            let Some(series) = self.series_entry(state.series_id) else {
                continue;
            };
            found = true;
            visible &= series.visible;
        }
        found.then_some(visible)
    }

    pub fn set_external_study_visible(&mut self, study_id: u64, visible: bool) -> bool {
        let series_ids = self
            .external_study_outputs
            .iter()
            .filter_map(|(&(candidate, _), state)| {
                (candidate == study_id).then_some(state.series_id)
            })
            .collect::<Vec<_>>();
        let changed = series_ids.iter().any(|&id| {
            self.series_entry(id)
                .is_some_and(|series| series.visible != visible)
        });
        for id in series_ids {
            self.set_series_visible(id, visible);
        }
        changed
    }

    pub fn remove_external_studies(&mut self, study_ids: &[u64]) -> bool {
        if study_ids.is_empty() {
            return false;
        }
        let keys = self
            .external_study_outputs
            .keys()
            .copied()
            .filter(|(study_id, _)| study_ids.contains(study_id))
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return false;
        }
        for key in keys {
            if let Some(state) = self.external_study_outputs.remove(&key) {
                if self.selected_series() == Some(state.series_id) {
                    self.set_selected_series(None);
                }
                let _ = self.remove_series(state.series_id);
            }
        }
        self.external_study_panes
            .retain(|(study_id, _), _| !study_ids.contains(study_id));
        true
    }

    pub(crate) fn apply_indicator_chrome_to_external_studies(
        &mut self,
        options: IndicatorChromeOptions,
    ) -> bool {
        let ids = self
            .external_study_outputs
            .values()
            .map(|state| state.series_id)
            .collect::<Vec<_>>();
        let mut changed = false;
        for id in ids {
            if let Some(series) = self.series_entry_mut(id) {
                changed |= series.title_visible != options.name_labels_visible
                    || series.last_value_visible != options.value_labels_visible
                    || series.price_line_visible != options.price_lines_visible;
                series.title_visible = options.name_labels_visible;
                series.last_value_visible = options.value_labels_visible;
                series.price_line_visible = options.price_lines_visible;
            }
        }
        changed
    }
}

fn output_info(
    study_id: u64,
    output_index: usize,
    state: &ExternalStudyOutputState,
) -> ExternalStudyOutputInfo {
    ExternalStudyOutputInfo {
        study_id,
        output_index,
        series_id: state.series_id,
        generation: state.generation,
        settings_available: state.settings_available,
        legend_label: state.legend_label.clone(),
        input_requirements: state.input_requirements,
    }
}

fn validate_descriptor(
    descriptor: ExternalStudyOutputDescriptor<'_>,
) -> Result<(), ExternalStudyError> {
    if descriptor.title.len() > MAX_EXTERNAL_STUDY_TITLE_BYTES
        || descriptor
            .legend_label
            .is_some_and(|label| label.len() > MAX_EXTERNAL_STUDY_LEGEND_LABEL_BYTES)
    {
        return Err(ExternalStudyError::MetadataTooLong);
    }
    if descriptor.threshold_region.is_some_and(|region| {
        !region.lower.is_finite()
            || !region.upper.is_finite()
            || region.lower >= region.upper
            || !matches!(
                descriptor.plot,
                ExternalStudyPlotKind::Line | ExternalStudyPlotKind::Area
            )
    }) || (descriptor.point_style == ExternalStudyPointStyle::MomentumHistogram
        && descriptor.plot != ExternalStudyPlotKind::Histogram)
    {
        return Err(ExternalStudyError::InvalidPresentation);
    }
    Ok(())
}

fn prepare_columns(
    timestamps_unix_nanos: &[i64],
    values: &[Option<f64>],
) -> Result<PreparedColumns, ExternalStudyError> {
    if timestamps_unix_nanos.len() != values.len() {
        return Err(ExternalStudyError::LengthMismatch);
    }
    let mut times = Vec::with_capacity(timestamps_unix_nanos.len());
    let mut scalar = Vec::with_capacity(values.len());
    let mut previous_second = None;
    for (&timestamp, value) in timestamps_unix_nanos.iter().zip(values) {
        if timestamp.rem_euclid(NANOS_PER_SECOND) != 0 {
            return Err(ExternalStudyError::UnsupportedTimestampPrecision);
        }
        let second = timestamp.div_euclid(NANOS_PER_SECOND);
        let time = second as f64;
        if validate_timestamp(time).is_err() || time as i64 != second {
            return Err(ExternalStudyError::UnsupportedTimestampPrecision);
        }
        if previous_second.is_some_and(|previous| previous >= second) {
            return Err(ExternalStudyError::NonIncreasingTimestamp);
        }
        previous_second = Some(second);
        times.push(time);
        scalar.push(match value {
            None => f64::NAN,
            Some(value) if value.is_finite() && value.abs() <= MAX_SAFE_VALUE => *value,
            Some(_) => return Err(ExternalStudyError::InvalidValue),
        });
    }
    Ok(PreparedColumns { times, scalar })
}

fn set_series_data(
    engine: &mut ChartEngine,
    series_id: SeriesId,
    columns: &PreparedColumns,
) -> Result<(), ExternalStudyError> {
    engine
        .set_series_data(
            series_id,
            &columns.times,
            &columns.scalar,
            &columns.scalar,
            &columns.scalar,
            &columns.scalar,
        )
        .map(|_| ())
        .map_err(|_| ExternalStudyError::InstallationRejected)
}

fn apply_presentation(
    engine: &mut ChartEngine,
    series_id: SeriesId,
    output_index: usize,
    descriptor: ExternalStudyOutputDescriptor<'_>,
) {
    if let Some(series) = engine.series_entry_mut(series_id) {
        series.title = descriptor.title.to_owned();
        series.line_color = Some(
            EMA_RIBBON_DEFAULT_COLORS[output_index % EMA_RIBBON_DEFAULT_COLORS.len()].to_owned(),
        );
    }
    let threshold = descriptor
        .threshold_region
        .map(|region| SeriesThresholdRegion {
            lower: region.lower,
            upper: region.upper,
        });
    let _ = engine.set_series_threshold_region(series_id, threshold);
    if descriptor.point_style == ExternalStudyPointStyle::MomentumHistogram {
        let _ = engine.apply_momentum_histogram_colors(series_id);
    }
}

const fn series_kind(plot: ExternalStudyPlotKind) -> SeriesKind {
    match plot {
        ExternalStudyPlotKind::Line => SeriesKind::Line,
        ExternalStudyPlotKind::Histogram => SeriesKind::Histogram,
        ExternalStudyPlotKind::Area => SeriesKind::Area,
    }
}

const fn scale_id(scale: ExternalStudyScaleTarget) -> &'static str {
    match scale {
        ExternalStudyScaleTarget::Primary => "right",
        ExternalStudyScaleTarget::Left => "left",
        ExternalStudyScaleTarget::Overlay => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor<'a>(
        title: &'a str,
        pane: ExternalStudyPaneTarget,
    ) -> ExternalStudyOutputDescriptor<'a> {
        ExternalStudyOutputDescriptor {
            title,
            legend_label: None,
            plot: ExternalStudyPlotKind::Line,
            pane,
            scale: ExternalStudyScaleTarget::Primary,
            settings_available: true,
            threshold_region: None,
            point_style: ExternalStudyPointStyle::Uniform,
            input_requirements: ExternalStudyInputRequirements::BARS
                .with(ExternalStudyInputStream::Trades),
        }
    }

    #[test]
    fn external_study_transaction_owns_validation_generation_and_groups() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = [1_000_000_000, 2_000_000_000];
        let values = [Some(1.0), Some(2.0)];
        let pane = ExternalStudyPaneTarget::Dedicated { group: 3 };
        assert_eq!(
            chart.install_external_study_output(
                7,
                0,
                descriptor("Signal", pane),
                1,
                &times,
                &values
            ),
            Ok(true)
        );
        assert_eq!(
            chart.install_external_study_output(
                7,
                1,
                descriptor("Average", pane),
                1,
                &times,
                &values
            ),
            Ok(true)
        );
        let outputs = chart.external_study_outputs();
        assert_eq!(outputs.len(), 2);
        assert!(chart.has_external_studies());
        assert_eq!(
            outputs[0].input_requirements,
            descriptor("", pane).input_requirements
        );
        let first = chart.series_entry(outputs[0].series_id).unwrap();
        let second = chart.series_entry(outputs[1].series_id).unwrap();
        assert_ne!(first.pane_index, 0);
        assert_eq!(first.pane_index, second.pane_index);
        assert_eq!(
            chart.external_study_for_series(outputs[1].series_id),
            Some(7)
        );
        chart.set_selected_series(Some(outputs[1].series_id));
        assert_eq!(chart.selected_series(), Some(outputs[1].series_id));
        assert_eq!(
            chart.selected_series_members().collect::<Vec<_>>(),
            outputs
                .iter()
                .map(|output| output.series_id)
                .collect::<Vec<_>>()
        );

        assert_eq!(
            chart.install_external_study_output(
                7,
                0,
                descriptor("Signal", pane),
                1,
                &times,
                &[Some(9.0), Some(9.0)]
            ),
            Ok(false),
            "a duplicate generation must not replace current data"
        );
        assert!(chart.set_external_study_visible(7, false));
        assert_eq!(chart.external_study_visible(7), Some(false));
        assert!(chart.remove_external_studies(&[7]));
        assert!(chart.external_study_outputs().is_empty());
        assert!(!chart.has_external_studies());
        assert_eq!(chart.external_study_for_series(outputs[0].series_id), None);
    }

    #[test]
    fn invalid_external_study_input_is_atomic_and_chrome_is_retained() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let chrome = IndicatorChromeOptions {
            name_labels_visible: false,
            value_labels_visible: false,
            price_lines_visible: false,
        };
        assert!(chart.set_indicator_chrome_options(chrome));
        assert_eq!(
            chart.install_external_study_output(
                8,
                0,
                descriptor("Bad", ExternalStudyPaneTarget::Price),
                1,
                &[1_000_000_001],
                &[Some(1.0)],
            ),
            Err(ExternalStudyError::UnsupportedTimestampPrecision)
        );
        assert!(chart.external_study_outputs().is_empty());

        assert_eq!(
            chart.install_external_study_output(
                8,
                0,
                descriptor("Good", ExternalStudyPaneTarget::Price),
                2,
                &[1_000_000_000],
                &[Some(1.0)],
            ),
            Ok(true)
        );
        let output = chart.external_study_outputs().pop().unwrap();
        let series = chart.series_entry(output.series_id).unwrap();
        assert!(!series.title_visible);
        assert!(!series.last_value_visible);
        assert!(!series.price_line_visible);
    }
}
