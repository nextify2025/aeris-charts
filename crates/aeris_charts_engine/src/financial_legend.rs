//! Engine-owned financial legend grouping and value projection.

use crate::{ChartEngine, SeriesId, SeriesValueSnapshot};
use std::collections::HashSet;

const MAX_HOST_LEGEND_SERIES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FinancialLegendIdentity {
    Primary,
    Host(u64),
    Indicator(SeriesId),
    ExternalStudy(u64),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FinancialLegendTone {
    #[default]
    Neutral,
    Bullish,
    Bearish,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinancialLegendValue {
    pub text: String,
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinancialLegendRow {
    pub identity: FinancialLegendIdentity,
    pub first_series_id: SeriesId,
    pub pane: usize,
    pub title: String,
    pub values: Vec<FinancialLegendValue>,
    pub tone: FinancialLegendTone,
    pub visible: bool,
    pub settings_available: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct HostLegendSeries<'a> {
    pub identity: u64,
    pub series_id: SeriesId,
    pub title: &'a str,
    pub settings_available: bool,
    /// Use the primary OHLC direction for product series whose value follows the active bar.
    pub tone_from_primary: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FinancialLegendRequest<'a> {
    pub logical_index: Option<i64>,
    pub primary_title: &'a str,
    pub show_primary_ohlc: bool,
    /// Product series placed before native indicator groups, such as volume.
    pub leading_series: &'a [HostLegendSeries<'a>],
    /// Product series placed after native indicator groups, such as order-flow studies.
    pub trailing_series: &'a [HostLegendSeries<'a>],
}

impl ChartEngine {
    /// Produce one ordered financial legend model from canonical series, indicator bindings,
    /// external-study groups, and a bounded list of explicitly product-owned series roles.
    #[must_use]
    pub fn financial_legend(&self, request: FinancialLegendRequest<'_>) -> Vec<FinancialLegendRow> {
        let snapshots = self.value_snapshot(request.logical_index);
        let mut rows = Vec::new();
        if let Some(primary) = self.series_entry(0) {
            let snapshot = snapshot_for(&snapshots, 0);
            rows.push(FinancialLegendRow {
                identity: FinancialLegendIdentity::Primary,
                first_series_id: 0,
                pane: primary.pane_index,
                title: if request.primary_title.is_empty() {
                    if primary.title.is_empty() {
                        "Asset".to_string()
                    } else {
                        primary.title.clone()
                    }
                } else {
                    request.primary_title.to_string()
                },
                values: if primary.visible && request.show_primary_ohlc {
                    snapshot.map_or_else(Vec::new, ohlc_values)
                } else {
                    Vec::new()
                },
                tone: snapshot.map_or(FinancialLegendTone::Neutral, snapshot_tone),
                visible: primary.visible,
                settings_available: false,
            });
        }
        append_host_rows(self, &snapshots, request.leading_series, &mut rows);

        let mut emitted_bindings = HashSet::new();
        for &series_id in self.series_order() {
            let Some(info) = self.indicator_info(series_id) else {
                continue;
            };
            if !emitted_bindings.insert(info.binding_id) {
                continue;
            }
            let outputs = self
                .series_entries()
                .iter()
                .filter(|series| {
                    !series.removed
                        && self
                            .indicator_info(series.id)
                            .is_some_and(|output| output.binding_id == info.binding_id)
                })
                .collect::<Vec<_>>();
            let Some(first) = outputs.first() else {
                continue;
            };
            let visible = outputs.iter().any(|series| series.visible);
            let values = if visible {
                outputs
                    .iter()
                    .filter(|series| series.visible)
                    .filter_map(|series| {
                        let output = self.indicator_info(series.id)?;
                        let value = snapshot_value(&snapshots, series.id)?;
                        Some(FinancialLegendValue {
                            text: if output.output_count > 1 && output.kind != "ema_ribbon" {
                                format!("{} {value}", output.output_name)
                            } else {
                                value
                            },
                            color: Some(
                                series
                                    .line_color
                                    .clone()
                                    .unwrap_or_else(|| crate::DEFAULT_LINE_COLOR.to_css()),
                            ),
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            rows.push(FinancialLegendRow {
                identity: FinancialLegendIdentity::Indicator(info.binding_id),
                first_series_id: first.id,
                pane: first.pane_index,
                title: if info.kind == "ema_ribbon" {
                    "EMA Ribbon".to_string()
                } else {
                    first.title.clone()
                },
                values,
                tone: FinancialLegendTone::Neutral,
                visible,
                settings_available: false,
            });
        }

        append_host_rows(self, &snapshots, request.trailing_series, &mut rows);

        let outputs = self.external_study_outputs();
        let mut emitted_studies = HashSet::new();
        for output in &outputs {
            if !emitted_studies.insert(output.study_id) {
                continue;
            }
            let group = outputs
                .iter()
                .filter(|candidate| candidate.study_id == output.study_id)
                .filter_map(|candidate| {
                    self.series_entry(candidate.series_id)
                        .map(|series| (candidate, series))
                })
                .collect::<Vec<_>>();
            let Some((first_output, first_series)) = group.first().copied() else {
                continue;
            };
            let visible = group.iter().any(|(_, series)| series.visible);
            let values = if visible {
                group
                    .iter()
                    .filter(|(_, series)| series.visible)
                    .filter_map(|(candidate, series)| {
                        let value = snapshot_value(&snapshots, candidate.series_id)?;
                        Some(FinancialLegendValue {
                            text: candidate
                                .legend_label
                                .as_ref()
                                .map_or(value.clone(), |label| format!("{label} {value}")),
                            color: Some(
                                series
                                    .line_color
                                    .clone()
                                    .unwrap_or_else(|| crate::DEFAULT_LINE_COLOR.to_css()),
                            ),
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            rows.push(FinancialLegendRow {
                identity: FinancialLegendIdentity::ExternalStudy(output.study_id),
                first_series_id: first_output.series_id,
                pane: first_series.pane_index,
                title: first_series.title.clone(),
                values,
                tone: FinancialLegendTone::Neutral,
                visible,
                settings_available: group
                    .iter()
                    .any(|(candidate, _)| candidate.settings_available),
            });
        }
        rows
    }
}

fn append_host_rows(
    chart: &ChartEngine,
    snapshots: &[SeriesValueSnapshot],
    descriptors: &[HostLegendSeries<'_>],
    rows: &mut Vec<FinancialLegendRow>,
) {
    for descriptor in descriptors.iter().take(MAX_HOST_LEGEND_SERIES) {
        let Some(series) = chart.series_entry(descriptor.series_id) else {
            continue;
        };
        rows.push(FinancialLegendRow {
            identity: FinancialLegendIdentity::Host(descriptor.identity),
            first_series_id: series.id,
            pane: series.pane_index,
            title: descriptor.title.to_string(),
            values: if series.visible {
                snapshot_value(snapshots, series.id).map_or_else(Vec::new, |text| {
                    vec![FinancialLegendValue { text, color: None }]
                })
            } else {
                Vec::new()
            },
            tone: if descriptor.tone_from_primary {
                snapshot_for(snapshots, 0).map_or(FinancialLegendTone::Neutral, snapshot_tone)
            } else {
                FinancialLegendTone::Neutral
            },
            visible: series.visible,
            settings_available: descriptor.settings_available,
        });
    }
}

fn snapshot_for(
    snapshots: &[SeriesValueSnapshot],
    series_id: SeriesId,
) -> Option<&SeriesValueSnapshot> {
    snapshots
        .iter()
        .find(|snapshot| snapshot.series_id == series_id)
}

fn snapshot_value(snapshots: &[SeriesValueSnapshot], series_id: SeriesId) -> Option<String> {
    let snapshot = snapshot_for(snapshots, series_id)?;
    snapshot
        .formatted_value
        .as_ref()
        .or(snapshot.formatted_close.as_ref())
        .filter(|value| !value.is_empty())
        .cloned()
}

fn ohlc_values(snapshot: &SeriesValueSnapshot) -> Vec<FinancialLegendValue> {
    [
        ("O", &snapshot.formatted_open),
        ("H", &snapshot.formatted_high),
        ("L", &snapshot.formatted_low),
        ("C", &snapshot.formatted_close),
    ]
    .into_iter()
    .map(|(label, value)| FinancialLegendValue {
        text: format!("{label} {}", value.as_deref().unwrap_or("--")),
        color: None,
    })
    .collect()
}

fn snapshot_tone(snapshot: &SeriesValueSnapshot) -> FinancialLegendTone {
    match (snapshot.open, snapshot.close) {
        (Some(open), Some(close)) if close >= open => FinancialLegendTone::Bullish,
        (Some(_), Some(_)) => FinancialLegendTone::Bearish,
        _ => FinancialLegendTone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SeriesKind;

    #[test]
    fn financial_legend_groups_primary_host_series_and_native_outputs() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart
            .set_series_data(
                0,
                &[1.0, 2.0, 3.0],
                &[10.0, 11.0, 12.0],
                &[12.0, 13.0, 14.0],
                &[9.0, 10.0, 11.0],
                &[11.0, 12.0, 13.0],
            )
            .unwrap();
        let volume = chart.add_series(SeriesKind::Histogram);
        chart
            .set_series_data(
                volume,
                &[1.0, 2.0, 3.0],
                &[100.0, 120.0, 140.0],
                &[100.0, 120.0, 140.0],
                &[100.0, 120.0, 140.0],
                &[100.0, 120.0, 140.0],
            )
            .unwrap();
        let sma = chart.add_sma(0, 2).unwrap();
        let request = FinancialLegendRequest {
            logical_index: None,
            primary_title: "BTCUSD",
            show_primary_ohlc: true,
            leading_series: &[HostLegendSeries {
                identity: 7,
                series_id: volume,
                title: "Volume",
                settings_available: false,
                tone_from_primary: true,
            }],
            trailing_series: &[],
        };

        let rows = chart.financial_legend(request);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].identity, FinancialLegendIdentity::Primary);
        assert_eq!(rows[0].title, "BTCUSD");
        assert_eq!(rows[0].values.len(), 4);
        assert_eq!(rows[0].tone, FinancialLegendTone::Bullish);
        assert_eq!(rows[1].identity, FinancialLegendIdentity::Host(7));
        assert_eq!(rows[1].first_series_id, volume);
        assert_eq!(rows[2].identity, FinancialLegendIdentity::Indicator(sma));
    }

    #[test]
    fn host_legend_descriptors_are_bounded() {
        let chart = ChartEngine::new(800.0, 500.0, 1.0);
        let descriptors = (0..32)
            .map(|identity| HostLegendSeries {
                identity,
                series_id: 0,
                title: "Host",
                settings_available: false,
                tone_from_primary: false,
            })
            .collect::<Vec<_>>();
        let rows = chart.financial_legend(FinancialLegendRequest {
            logical_index: None,
            primary_title: "",
            show_primary_ohlc: false,
            leading_series: &descriptors,
            trailing_series: &[],
        });
        assert_eq!(rows.len(), 1 + MAX_HOST_LEGEND_SERIES);
    }
}
