//! Engine-owned visible-range OHLCV volume-profile indicator.
use crate::native_primitives::NativeSeriesPrimitiveKind;
use crate::{ChartEngine, NativePrimitiveId, SeriesId};
use aeris_charts_indicators::volume_profile::{
    volume_profile, ProfileBar, VolumeProfile, MAX_VOLUME_PROFILE_ROWS,
};
use aeris_charts_render::color::Color;

pub const MAX_VOLUME_PROFILE_INDICATORS: usize = 16;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VolumeProfileIndicatorOptions {
    pub rows: usize,
    pub value_area_percent: f64,
    pub width_percent: f64,
    pub visible: bool,
    pub show_poc: bool,
    pub show_value_area: bool,
    pub up_color: String,
    pub down_color: String,
    pub value_area_up_color: String,
    pub value_area_down_color: String,
    pub poc_color: String,
}
impl Default for VolumeProfileIndicatorOptions {
    fn default() -> Self {
        Self {
            rows: 48,
            value_area_percent: 70.0,
            width_percent: 25.0,
            visible: true,
            show_poc: true,
            show_value_area: true,
            up_color: "rgba(8,153,129,0.45)".into(),
            down_color: "rgba(247,82,95,0.45)".into(),
            value_area_up_color: "rgba(8,153,129,0.78)".into(),
            value_area_down_color: "rgba(247,82,95,0.78)".into(),
            poc_color: "#f5a623".into(),
        }
    }
}
impl VolumeProfileIndicatorOptions {
    fn valid(&self) -> bool {
        (1..=MAX_VOLUME_PROFILE_ROWS).contains(&self.rows)
            && self.value_area_percent.is_finite()
            && self.value_area_percent > 0.0
            && self.value_area_percent <= 100.0
            && self.width_percent.is_finite()
            && self.width_percent > 0.0
            && self.width_percent <= 50.0
            && [
                &self.up_color,
                &self.down_color,
                &self.value_area_up_color,
                &self.value_area_down_color,
                &self.poc_color,
            ]
            .iter()
            .all(|color| color.len() <= 128 && Color::parse_css(color).is_some())
    }
}

#[derive(Clone, Debug, Default)]
pub struct VolumeProfileIndicatorSnapshot {
    pub profile: VolumeProfile,
    pub error: Option<&'static str>,
    pub calculation_revision: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct VolumeProfileIndicatorState {
    pub volume_source: SeriesId,
    pub options: VolumeProfileIndicatorOptions,
    pub snapshot: VolumeProfileIndicatorSnapshot,
    key: Option<[u64; 5]>,
}
impl VolumeProfileIndicatorState {
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.snapshot.profile.rows.capacity()
            * core::mem::size_of::<aeris_charts_indicators::volume_profile::ProfileRow>()
            + self.options.up_color.capacity()
            + self.options.down_color.capacity()
            + self.options.value_area_up_color.capacity()
            + self.options.value_area_down_color.capacity()
            + self.options.poc_color.capacity()
    }
}

impl ChartEngine {
    /// Bind a visible-range distribution to OHLC prices and timestamp-aligned scalar volume.
    /// Unlike line indicators it owns price bins, so it returns an indicator handle rather than
    /// a time-series output. Both dependencies must be live data series.
    pub fn add_volume_profile_indicator(
        &mut self,
        source: SeriesId,
        volume_source: SeriesId,
        options: VolumeProfileIndicatorOptions,
    ) -> Option<NativePrimitiveId> {
        let price = self.series_entry(source)?;
        let volume = self.series_entry(volume_source)?;
        if source == volume_source
            || !options.valid()
            || !matches!(
                price.kind,
                crate::SeriesKind::Candlestick | crate::SeriesKind::Bar
            )
            || !matches!(
                volume.kind,
                crate::SeriesKind::Line
                    | crate::SeriesKind::Histogram
                    | crate::SeriesKind::Area
                    | crate::SeriesKind::Baseline
            )
            || self
                .series
                .iter()
                .flat_map(|series| &series.native_primitives)
                .filter(|primitive| {
                    matches!(
                        primitive.kind,
                        NativeSeriesPrimitiveKind::VolumeProfileIndicator(_)
                    )
                })
                .count()
                >= MAX_VOLUME_PROFILE_INDICATORS
        {
            return None;
        }
        self.insert_native_primitive(
            source,
            NativeSeriesPrimitiveKind::VolumeProfileIndicator(Box::new(
                VolumeProfileIndicatorState {
                    volume_source,
                    options,
                    snapshot: VolumeProfileIndicatorSnapshot::default(),
                    key: None,
                },
            )),
        )
    }

    pub fn set_volume_profile_indicator_options(
        &mut self,
        id: NativePrimitiveId,
        options: VolumeProfileIndicatorOptions,
    ) -> bool {
        if !options.valid() {
            return false;
        }
        let Some(source) = self
            .series
            .iter()
            .find(|series| {
                series.native_primitives.iter().any(|primitive| {
                    primitive.id == id
                        && matches!(
                            primitive.kind,
                            NativeSeriesPrimitiveKind::VolumeProfileIndicator(_)
                        )
                })
            })
            .map(|series| series.id)
        else {
            return false;
        };
        let Some(series) = self.series_entry_mut(source) else {
            return false;
        };
        let Some(primitive) = series
            .native_primitives
            .iter_mut()
            .find(|primitive| primitive.id == id)
        else {
            return false;
        };
        let NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) = &mut primitive.kind else {
            return false;
        };
        if state.options.rows != options.rows
            || state.options.value_area_percent != options.value_area_percent
        {
            state.key = None;
        }
        state.options = options;
        self.invalidate_frame_series(source);
        true
    }

    pub fn volume_profile_indicator_options(
        &self,
        id: NativePrimitiveId,
    ) -> Option<&VolumeProfileIndicatorOptions> {
        self.series
            .iter()
            .flat_map(|series| &series.native_primitives)
            .find_map(|primitive| {
                if primitive.id != id {
                    return None;
                }
                match &primitive.kind {
                    NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) => {
                        Some(&state.options)
                    }
                    _ => None,
                }
            })
    }

    pub fn volume_profile_indicator_snapshot(
        &mut self,
        id: NativePrimitiveId,
    ) -> Option<&VolumeProfileIndicatorSnapshot> {
        self.refresh_volume_profile_indicators();
        self.series
            .iter()
            .flat_map(|series| &series.native_primitives)
            .find_map(|primitive| {
                if primitive.id != id {
                    return None;
                }
                match &primitive.kind {
                    NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) => {
                        Some(&state.snapshot)
                    }
                    _ => None,
                }
            })
    }

    pub(crate) fn drop_volume_profiles_using(&mut self, volume_source: SeriesId) {
        let sources = self.series.iter().filter(|series| series.native_primitives.iter().any(|primitive| matches!(&primitive.kind, NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) if state.volume_source == volume_source))).map(|series| series.id).collect::<Vec<_>>();
        for source in sources {
            if let Some(series) = self.series_entry_mut(source) {
                series.native_primitives.retain(|primitive| !matches!(&primitive.kind, NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) if state.volume_source == volume_source));
            }
            self.invalidate_frame_series(source);
        }
    }

    pub(crate) fn refresh_volume_profile_indicators(&mut self) {
        let (from, to) = self.visible_range().unwrap_or((0, -1));
        let mut pending = Vec::new();
        for series in self.series.iter() {
            for primitive in &series.native_primitives {
                let NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) = &primitive.kind
                else {
                    continue;
                };
                let range = if to >= from {
                    let plot = self.data.plot(series.id);
                    plot.source_range(plot.visible_rows(from, to))
                } else {
                    0..0
                };
                let key = [
                    self.data.series_generation(series.id).unwrap_or(0),
                    self.data
                        .series_generation(state.volume_source)
                        .unwrap_or(0),
                    range.start as u64,
                    range.end as u64,
                    series.price_format.min_move.to_bits(),
                ];
                if state.key != Some(key) {
                    pending.push((
                        series.id,
                        primitive.id,
                        state.volume_source,
                        state.options.rows,
                        state.options.value_area_percent,
                        series.price_format.min_move,
                        key,
                    ));
                }
            }
        }
        for (source, id, volume_source, rows, area, minimum_span, key) in pending {
            let result = match (
                self.data.series_data(source),
                self.data.series_data(volume_source),
            ) {
                (Some((times, values)), Some((volume_times, volumes))) => {
                    // Canonical bars in view, each once (as-of points repeat rows).
                    let range = if to >= from {
                        let plot = self.data.plot(source);
                        plot.source_range(plot.visible_rows(from, to))
                    } else {
                        0..0
                    };
                    let mut volume_row = range.clone().next().map_or(0, |row| {
                        volume_times.partition_point(|time| *time < times[row])
                    });
                    let bars = range.map(move |row| {
                        while volume_row < volume_times.len()
                            && volume_times[volume_row] < times[row]
                        {
                            volume_row += 1;
                        }
                        let volume = if volume_times.get(volume_row) == Some(&times[row]) {
                            volumes[3][volume_row]
                        } else {
                            f64::NAN
                        };
                        ProfileBar {
                            open: values[0][row],
                            low: values[2][row],
                            high: values[1][row],
                            close: values[3][row],
                            volume: if values[3][row].is_finite() {
                                volume
                            } else {
                                f64::NAN
                            },
                        }
                    });
                    volume_profile(bars, rows, area, minimum_span)
                }
                _ => Err("volume-profile source is unavailable"),
            };
            if let Some(series) = self.series_entry_mut(source) {
                if let Some(primitive) = series
                    .native_primitives
                    .iter_mut()
                    .find(|primitive| primitive.id == id)
                {
                    if let NativeSeriesPrimitiveKind::VolumeProfileIndicator(state) =
                        &mut primitive.kind
                    {
                        state.key = Some(key);
                        state.snapshot.calculation_revision =
                            state.snapshot.calculation_revision.wrapping_add(1);
                        match result {
                            Ok(profile) => {
                                state.snapshot.profile = profile;
                                state.snapshot.error = None;
                            }
                            Err(error) => {
                                state.snapshot.profile = VolumeProfile::default();
                                state.snapshot.error = Some(error);
                            }
                        }
                    }
                }
            }
            self.invalidate_frame_series(source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SeriesKind;
    use aeris_charts_render::draw_list::Prim;

    fn fixture() -> (ChartEngine, SeriesId, NativePrimitiveId) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.5);
        chart
            .set_series_data(
                0,
                &[10.0, 20.0, 30.0],
                &[1.0; 3],
                &[4.0; 3],
                &[0.0; 3],
                &[2.0; 3],
            )
            .unwrap();
        let volume = chart.add_series(SeriesKind::Histogram);
        // Deliberately different timestamps: positional zipping would wrongly use 999.
        chart
            .set_series_data(
                volume,
                &[5.0, 10.0, 30.0],
                &[999.0, 40.0, 80.0],
                &[999.0, 40.0, 80.0],
                &[999.0, 40.0, 80.0],
                &[999.0, 40.0, 80.0],
            )
            .unwrap();
        chart.series_entry_mut(volume).unwrap().visible = false;
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let id = chart
            .add_volume_profile_indicator(
                0,
                volume,
                VolumeProfileIndicatorOptions {
                    rows: 4,
                    ..Default::default()
                },
            )
            .unwrap();
        (chart, volume, id)
    }

    #[test]
    fn real_frame_tracks_time_aligned_volume_range_updates_and_cache() {
        let (mut chart, volume, id) = fixture();
        let frame = chart.build_frame();
        let profile = chart.volume_profile_indicator_snapshot(id).unwrap();
        assert_eq!(profile.profile.total_volume, 120.0);
        assert_eq!(profile.profile.bar_count, 2);
        let revision = profile.calculation_revision;
        let options = VolumeProfileIndicatorOptions::default();
        let up_color = Color::parse_css(&options.value_area_up_color).unwrap();
        let base_up_color = Color::parse_css(&options.up_color).unwrap();
        let pane_right = frame.panes[0].scissor[2] as i32;
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::Rect { rect, color }
                if (*color == up_color || *color == base_up_color)
                    && rect.x + rect.w == pane_right
        )));
        assert!(!frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::HLine {
                style: crate::LineStyle::Dashed,
                ..
            }
        )));
        chart.build_frame();
        assert_eq!(
            chart
                .volume_profile_indicator_snapshot(id)
                .unwrap()
                .calculation_revision,
            revision
        );
        chart.update_series_bar(volume, 30.0, [160.0; 4]);
        chart.build_frame();
        assert_eq!(
            chart
                .volume_profile_indicator_snapshot(id)
                .unwrap()
                .profile
                .total_volume,
            200.0
        );
        chart.set_visible_logical_range(1.0, 2.0);
        chart.build_frame();
        assert_eq!(
            chart
                .volume_profile_indicator_snapshot(id)
                .unwrap()
                .profile
                .total_volume,
            40.0
        );
    }

    #[test]
    fn options_are_atomic_and_dependency_removal_invalidates_the_handle() {
        let (mut chart, volume, id) = fixture();
        let before = chart.volume_profile_indicator_options(id).unwrap().clone();
        assert!(!chart.set_volume_profile_indicator_options(
            id,
            VolumeProfileIndicatorOptions {
                rows: 513,
                ..before.clone()
            }
        ));
        assert_eq!(chart.volume_profile_indicator_options(id), Some(&before));
        chart.remove_series(volume);
        assert!(chart.volume_profile_indicator_snapshot(id).is_none());
        let (mut chart, _, id) = fixture();
        chart.remove_series(0);
        assert!(chart.volume_profile_indicator_snapshot(id).is_none());
    }
}
