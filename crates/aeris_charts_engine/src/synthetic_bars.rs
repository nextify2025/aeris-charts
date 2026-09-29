//! Deterministic price-action chart transforms.
//!
//! Hosts supply canonical OHLC bars. The engine owns the synthetic sequence and preserves each
//! output bar's source-time bounds; renderers consume ordinary series geometry and never rebuild
//! Renko, Line Break, Kagi, or Point & Figure state.

use crate::BarSequencePoint;

pub const MAX_SYNTHETIC_BARS: usize = 1_000_000;
pub const MAX_SYNTHETIC_SOURCE_BARS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyntheticBarOptions {
    RenkoFixed { box_size: f64 },
    RenkoAtr { period: u32 },
    LineBreak { lines: u32 },
    Kagi { reversal_size: f64 },
    PointAndFigure { box_size: f64, reversal_boxes: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntheticSourceBar {
    pub timestamp_micros: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct SyntheticBar {
    pub logical_index: u64,
    pub open_timestamp_micros: i64,
    pub close_timestamp_micros: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

impl SyntheticBar {
    pub(crate) fn sequence_point(self) -> BarSequencePoint {
        BarSequencePoint {
            logical_index: self.logical_index,
            open_timestamp_micros: self.open_timestamp_micros,
            close_timestamp_micros: self.close_timestamp_micros,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntheticBarError {
    InvalidOptions,
    InvalidSource { index: usize },
    OutOfOrderSource { index: usize },
    UnknownSeries(u32),
    UnsupportedSeries(u32),
    SequenceDomainInUse,
    Capacity,
    SourceCapacity,
}

impl core::fmt::Display for SyntheticBarError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidOptions => write!(f, "synthetic bar options are invalid"),
            Self::InvalidSource { index } => write!(f, "source bar {index} is invalid"),
            Self::OutOfOrderSource { index } => {
                write!(f, "source bar {index} is not ordered by timestamp")
            }
            Self::UnknownSeries(id) => write!(f, "unknown series {id}"),
            Self::UnsupportedSeries(id) => {
                write!(
                    f,
                    "series {id} must use candlestick or OHLC-bar presentation"
                )
            }
            Self::SequenceDomainInUse => write!(
                f,
                "a chart can own only one independent non-time bar sequence, and none beside \
                 time-resampled series"
            ),
            Self::Capacity => write!(
                f,
                "synthetic bar sequence exceeds the {MAX_SYNTHETIC_BARS} bar limit"
            ),
            Self::SourceCapacity => write!(
                f,
                "synthetic source exceeds the {MAX_SYNTHETIC_SOURCE_BARS} row limit"
            ),
        }
    }
}

impl std::error::Error for SyntheticBarError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

#[derive(Clone, Debug)]
pub struct SyntheticBarAggregator {
    options: SyntheticBarOptions,
    source: Vec<SyntheticSourceBar>,
    bars: Vec<SyntheticBar>,
    atr_values: Vec<Option<f64>>,
}

impl SyntheticBarAggregator {
    pub fn new(options: SyntheticBarOptions) -> Result<Self, SyntheticBarError> {
        validate_options(options)?;
        Ok(Self {
            options,
            source: Vec::new(),
            bars: Vec::new(),
            atr_values: Vec::new(),
        })
    }

    pub fn options(&self) -> SyntheticBarOptions {
        self.options
    }

    pub fn source(&self) -> &[SyntheticSourceBar] {
        &self.source
    }

    pub fn bars(&self) -> &[SyntheticBar] {
        &self.bars
    }

    pub fn set_source(&mut self, source: Vec<SyntheticSourceBar>) -> Result<(), SyntheticBarError> {
        validate_source(&source)?;
        let mut candidate = Self::new(self.options)?;
        candidate.source = source;
        candidate.rebuild()?;
        *self = candidate;
        Ok(())
    }

    pub fn update_source(&mut self, bar: SyntheticSourceBar) -> Result<(), SyntheticBarError> {
        validate_source(core::slice::from_ref(&bar))?;
        match self.source.last().map(|source| source.timestamp_micros) {
            Some(last) if bar.timestamp_micros < last => {
                return Err(SyntheticBarError::OutOfOrderSource {
                    index: self.source.len(),
                });
            }
            Some(last) if bar.timestamp_micros == last => {
                let mut candidate = self.clone();
                if let Some(current) = candidate.source.last_mut() {
                    *current = bar;
                }
                candidate.rebuild()?;
                *self = candidate;
            }
            _ => {
                if self.source.len() == MAX_SYNTHETIC_SOURCE_BARS {
                    return Err(SyntheticBarError::SourceCapacity);
                }
                let previous_bars_len = self.bars.len();
                let previous_last = self.bars.last().copied();
                let previous_atr_len = self.atr_values.len();
                self.source.push(bar);
                if let Err(error) = self.append_tip() {
                    self.source.pop();
                    self.atr_values.truncate(previous_atr_len);
                    self.bars.truncate(previous_bars_len);
                    if let (Some(previous), Some(current)) = (previous_last, self.bars.last_mut()) {
                        *current = previous;
                    }
                    return Err(error);
                }
                return Ok(());
            }
        }
        Ok(())
    }

    fn rebuild(&mut self) -> Result<(), SyntheticBarError> {
        self.atr_values.clear();
        self.bars = match self.options {
            SyntheticBarOptions::RenkoFixed { box_size } => {
                build_renko(&self.source, |_| Some(box_size))
            }
            SyntheticBarOptions::RenkoAtr { period } => {
                self.atr_values = atr_values(&self.source, period as usize);
                build_atr_renko_with_values(&self.source, &self.atr_values)
            }
            SyntheticBarOptions::LineBreak { lines } => {
                build_line_break(&self.source, lines as usize)
            }
            SyntheticBarOptions::Kagi { reversal_size } => build_kagi(&self.source, reversal_size),
            SyntheticBarOptions::PointAndFigure {
                box_size,
                reversal_boxes,
            } => build_point_and_figure(&self.source, box_size, reversal_boxes),
        }?;
        for (index, bar) in self.bars.iter_mut().enumerate() {
            bar.logical_index = index as u64;
        }
        Ok(())
    }

    fn append_tip(&mut self) -> Result<(), SyntheticBarError> {
        let index = self.source.len().saturating_sub(1);
        match self.options {
            SyntheticBarOptions::RenkoFixed { box_size } => {
                self.append_renko(index, box_size, 0)?;
            }
            SyntheticBarOptions::RenkoAtr { period } => {
                let period = period as usize;
                let range = true_range(&self.source, index);
                let next = if index + 1 == period {
                    Some(
                        (0..=index)
                            .map(|row| true_range(&self.source, row))
                            .sum::<f64>()
                            / period as f64,
                    )
                } else if index + 1 > period {
                    self.atr_values[index - 1]
                        .map(|previous| (previous * (period - 1) as f64 + range) / period as f64)
                } else {
                    None
                };
                self.atr_values.push(next);
                if let Some(box_size) = next {
                    self.append_renko(index, box_size, period - 1)?;
                }
            }
            SyntheticBarOptions::LineBreak { lines } => {
                self.append_line_break(index, lines as usize)?
            }
            SyntheticBarOptions::Kagi { reversal_size } => {
                self.append_kagi(index, reversal_size)?
            }
            SyntheticBarOptions::PointAndFigure {
                box_size,
                reversal_boxes,
            } => self.append_point_and_figure(index, box_size, reversal_boxes)?,
        }
        Ok(())
    }

    fn append_renko(
        &mut self,
        index: usize,
        box_size: f64,
        anchor_index: usize,
    ) -> Result<(), SyntheticBarError> {
        if index <= anchor_index {
            return Ok(());
        }
        let source = self.source[index];
        let mut anchor = self
            .bars
            .last()
            .map_or(self.source[anchor_index].close, |bar| bar.close);
        let mut anchor_time = self
            .bars
            .last()
            .map_or(self.source[anchor_index].timestamp_micros, |bar| {
                bar.close_timestamp_micros
            });
        let mut direction = self.bars.last().map(|bar| {
            if bar.close >= bar.open {
                Direction::Up
            } else {
                Direction::Down
            }
        });
        loop {
            let up_distance = source.close - anchor;
            let down_distance = anchor - source.close;
            let next = match direction {
                None if up_distance >= box_size => Some((Direction::Up, anchor, anchor + box_size)),
                None if down_distance >= box_size => {
                    Some((Direction::Down, anchor, anchor - box_size))
                }
                Some(Direction::Up) if up_distance >= box_size => {
                    Some((Direction::Up, anchor, anchor + box_size))
                }
                Some(Direction::Up) if down_distance >= box_size * 2.0 => {
                    Some((Direction::Down, anchor - box_size, anchor - box_size * 2.0))
                }
                Some(Direction::Down) if down_distance >= box_size => {
                    Some((Direction::Down, anchor, anchor - box_size))
                }
                Some(Direction::Down) if up_distance >= box_size * 2.0 => {
                    Some((Direction::Up, anchor + box_size, anchor + box_size * 2.0))
                }
                _ => None,
            };
            let Some((next_direction, open, close)) = next else {
                break;
            };
            output(
                &mut self.bars,
                anchor_time,
                source.timestamp_micros,
                open,
                close,
            )?;
            anchor = close;
            anchor_time = source.timestamp_micros;
            direction = Some(next_direction);
        }
        Ok(())
    }

    fn append_line_break(&mut self, index: usize, lines: usize) -> Result<(), SyntheticBarError> {
        let source = self.source[index];
        let Some(previous) = self.bars.last().copied() else {
            output(
                &mut self.bars,
                source.timestamp_micros,
                source.timestamp_micros,
                source.open,
                source.close,
            )?;
            return Ok(());
        };
        let from = self.bars.len().saturating_sub(lines);
        let recent = &self.bars[from..];
        let high = recent
            .iter()
            .map(|bar| bar.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let low = recent
            .iter()
            .map(|bar| bar.low)
            .fold(f64::INFINITY, f64::min);
        if source.close > high || source.close < low {
            output(
                &mut self.bars,
                previous.close_timestamp_micros,
                source.timestamp_micros,
                previous.close,
                source.close,
            )?;
        }
        Ok(())
    }

    fn append_kagi(&mut self, index: usize, reversal_size: f64) -> Result<(), SyntheticBarError> {
        if index == 0 {
            return Ok(());
        }
        let source = self.source[index];
        let Some(last) = self.bars.last().copied() else {
            let first = self.source[0];
            if (source.close - first.close).abs() >= reversal_size {
                output(
                    &mut self.bars,
                    first.timestamp_micros,
                    source.timestamp_micros,
                    first.close,
                    source.close,
                )?;
            }
            return Ok(());
        };
        let rising = last.close >= last.open;
        if (rising && source.close > last.close) || (!rising && source.close < last.close) {
            if let Some(current) = self.bars.last_mut() {
                current.close = source.close;
                current.high = current.high.max(source.close);
                current.low = current.low.min(source.close);
                current.close_timestamp_micros = source.timestamp_micros;
            }
        } else if (rising && source.close <= last.close - reversal_size)
            || (!rising && source.close >= last.close + reversal_size)
        {
            output(
                &mut self.bars,
                last.close_timestamp_micros,
                source.timestamp_micros,
                last.close,
                source.close,
            )?;
        }
        Ok(())
    }

    fn append_point_and_figure(
        &mut self,
        index: usize,
        box_size: f64,
        reversal_boxes: u32,
    ) -> Result<(), SyntheticBarError> {
        if index == 0 {
            return Ok(());
        }
        let source = self.source[index];
        let reversal = box_size * f64::from(reversal_boxes);
        let Some(last) = self.bars.last().copied() else {
            let first = self.source[0];
            let distance = source.close - first.close;
            if distance.abs() >= box_size {
                let boxes = (distance.abs() / box_size).floor();
                let close = first.close + distance.signum() * boxes * box_size;
                output(
                    &mut self.bars,
                    first.timestamp_micros,
                    source.timestamp_micros,
                    first.close,
                    close,
                )?;
            }
            return Ok(());
        };
        let rising = last.close >= last.open;
        if rising && source.close >= last.close + box_size {
            let close = last.close + ((source.close - last.close) / box_size).floor() * box_size;
            if let Some(current) = self.bars.last_mut() {
                current.close = close;
                current.high = close;
                current.close_timestamp_micros = source.timestamp_micros;
            }
        } else if !rising && source.close <= last.close - box_size {
            let close = last.close - ((last.close - source.close) / box_size).floor() * box_size;
            if let Some(current) = self.bars.last_mut() {
                current.close = close;
                current.low = close;
                current.close_timestamp_micros = source.timestamp_micros;
            }
        } else if rising && source.close <= last.close - reversal {
            let close = last.close - ((last.close - source.close) / box_size).floor() * box_size;
            output(
                &mut self.bars,
                last.close_timestamp_micros,
                source.timestamp_micros,
                last.close - box_size,
                close,
            )?;
        } else if !rising && source.close >= last.close + reversal {
            let close = last.close + ((source.close - last.close) / box_size).floor() * box_size;
            output(
                &mut self.bars,
                last.close_timestamp_micros,
                source.timestamp_micros,
                last.close + box_size,
                close,
            )?;
        }
        Ok(())
    }

    fn bars_at(&self, clock_micros: Option<i64>) -> Result<Vec<SyntheticBar>, SyntheticBarError> {
        let Some(clock_micros) = clock_micros else {
            return Ok(self.bars.clone());
        };
        let visible = self
            .source
            .partition_point(|bar| bar.timestamp_micros <= clock_micros);
        let mut projection = Self::new(self.options)?;
        projection.set_source(self.source[..visible].to_vec())?;
        Ok(projection.bars)
    }
}

impl crate::ChartEngine {
    pub fn configure_synthetic_bar_series(
        &mut self,
        id: crate::SeriesId,
        options: SyntheticBarOptions,
    ) -> Result<(), SyntheticBarError> {
        let series = self
            .series_entry(id)
            .ok_or(SyntheticBarError::UnknownSeries(id))?;
        if !matches!(
            series.kind,
            crate::SeriesKind::Candlestick | crate::SeriesKind::Bar
        ) {
            return Err(SyntheticBarError::UnsupportedSeries(id));
        }
        let other_synthetic = self
            .synthetic_series
            .keys()
            .any(|&series_id| series_id != id);
        let non_time_stream = self.trade_streams.values().any(|stream| {
            !matches!(
                stream.options().bars,
                crate::FootprintBarAggregation::Time { .. }
            )
        });
        if other_synthetic || non_time_stream || !self.resampled_series.is_empty() {
            return Err(SyntheticBarError::SequenceDomainInUse);
        }
        let aggregator = SyntheticBarAggregator::new(options)?;
        // Synthetic bars live on the non-time sequence axis; an as-of overlay rejoins the union.
        self.rejoin_time_union(id);
        self.synthetic_series.insert(id, aggregator);
        self.refresh_synthetic_bar_projection(id)
    }

    pub fn set_synthetic_bar_source(
        &mut self,
        id: crate::SeriesId,
        source: Vec<SyntheticSourceBar>,
    ) -> Result<(), SyntheticBarError> {
        self.synthetic_series
            .get_mut(&id)
            .ok_or(SyntheticBarError::UnknownSeries(id))?
            .set_source(source)?;
        self.refresh_synthetic_bar_projection(id)
    }

    pub fn update_synthetic_bar_source(
        &mut self,
        id: crate::SeriesId,
        source: SyntheticSourceBar,
    ) -> Result<(), SyntheticBarError> {
        let (previous_len, previous_last) = {
            let aggregator = self
                .synthetic_series
                .get(&id)
                .ok_or(SyntheticBarError::UnknownSeries(id))?;
            (aggregator.bars().len(), aggregator.bars().last().copied())
        };
        self.synthetic_series
            .get_mut(&id)
            .ok_or(SyntheticBarError::UnknownSeries(id))?
            .update_source(source)?;
        let (next_len, next_last) = {
            let aggregator = self
                .synthetic_series
                .get(&id)
                .ok_or(SyntheticBarError::UnknownSeries(id))?;
            (aggregator.bars().len(), aggregator.bars().last().copied())
        };
        if previous_len == next_len && previous_last == next_last {
            return Ok(());
        }
        if self.replay_clock_micros().is_none() {
            let from = previous_len.saturating_sub(1).min(next_len);
            return self.refresh_synthetic_bar_projection_from(id, from);
        }
        self.refresh_synthetic_bar_projection(id)
    }

    pub fn synthetic_bars(&self, id: crate::SeriesId) -> Option<&[SyntheticBar]> {
        Some(self.synthetic_series.get(&id)?.bars())
    }

    pub(crate) fn synthetic_bar_options(&self, id: crate::SeriesId) -> Option<SyntheticBarOptions> {
        Some(self.synthetic_series.get(&id)?.options())
    }

    pub(crate) fn refresh_synthetic_replay_projections(&mut self) -> Result<(), SyntheticBarError> {
        let ids = self.synthetic_series.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.refresh_synthetic_bar_projection(id)?;
        }
        Ok(())
    }

    fn refresh_synthetic_bar_projection(
        &mut self,
        id: crate::SeriesId,
    ) -> Result<(), SyntheticBarError> {
        let bars = self
            .synthetic_series
            .get(&id)
            .ok_or(SyntheticBarError::UnknownSeries(id))?
            .bars_at(self.replay_clock_micros())?;
        if self.install_sequence_projection_inner(id, None, projection_columns(&bars)) {
            Ok(())
        } else {
            Err(SyntheticBarError::InvalidSource { index: 0 })
        }
    }

    fn refresh_synthetic_bar_projection_from(
        &mut self,
        id: crate::SeriesId,
        from: usize,
    ) -> Result<(), SyntheticBarError> {
        let (projection, expected) = {
            let bars = self
                .synthetic_series
                .get(&id)
                .ok_or(SyntheticBarError::UnknownSeries(id))?
                .bars();
            if from >= bars.len() {
                return Ok(());
            }
            (projection_columns(&bars[from..]), bars.len() - from)
        };
        if self.update_sequence_projection_bars_inner(id, from, projection) == expected {
            Ok(())
        } else {
            Err(SyntheticBarError::InvalidSource { index: from })
        }
    }
}

fn projection_columns(bars: &[SyntheticBar]) -> crate::SequenceProjectionColumns {
    (
        bars.iter()
            .copied()
            .map(SyntheticBar::sequence_point)
            .collect(),
        bars.iter().map(|bar| bar.open).collect(),
        bars.iter().map(|bar| bar.high).collect(),
        bars.iter().map(|bar| bar.low).collect(),
        bars.iter().map(|bar| bar.close).collect(),
    )
}

fn validate_options(options: SyntheticBarOptions) -> Result<(), SyntheticBarError> {
    let positive = |value: f64| value.is_finite() && value > 0.0;
    let valid = match options {
        SyntheticBarOptions::RenkoFixed { box_size } => positive(box_size),
        SyntheticBarOptions::RenkoAtr { period } => period > 0,
        SyntheticBarOptions::LineBreak { lines } => lines > 0,
        SyntheticBarOptions::Kagi { reversal_size } => positive(reversal_size),
        SyntheticBarOptions::PointAndFigure {
            box_size,
            reversal_boxes,
        } => positive(box_size) && reversal_boxes > 0,
    };
    if valid {
        Ok(())
    } else {
        Err(SyntheticBarError::InvalidOptions)
    }
}

fn validate_source(source: &[SyntheticSourceBar]) -> Result<(), SyntheticBarError> {
    if source.len() > MAX_SYNTHETIC_SOURCE_BARS {
        return Err(SyntheticBarError::SourceCapacity);
    }
    for (index, bar) in source.iter().enumerate() {
        if ![bar.open, bar.high, bar.low, bar.close]
            .iter()
            .all(|value| value.is_finite())
            || bar.high < bar.open.max(bar.close)
            || bar.low > bar.open.min(bar.close)
            || bar.high < bar.low
        {
            return Err(SyntheticBarError::InvalidSource { index });
        }
        if index > 0 && bar.timestamp_micros <= source[index - 1].timestamp_micros {
            return Err(SyntheticBarError::OutOfOrderSource { index });
        }
    }
    Ok(())
}

fn output(
    bars: &mut Vec<SyntheticBar>,
    open_time: i64,
    close_time: i64,
    open: f64,
    close: f64,
) -> Result<(), SyntheticBarError> {
    if bars.len() == MAX_SYNTHETIC_BARS {
        return Err(SyntheticBarError::Capacity);
    }
    bars.push(SyntheticBar {
        logical_index: bars.len() as u64,
        open_timestamp_micros: open_time,
        close_timestamp_micros: close_time,
        open,
        high: open.max(close),
        low: open.min(close),
        close,
    });
    Ok(())
}

fn build_renko(
    source: &[SyntheticSourceBar],
    mut box_at: impl FnMut(usize) -> Option<f64>,
) -> Result<Vec<SyntheticBar>, SyntheticBarError> {
    let mut bars = Vec::new();
    let Some(first) = source.first() else {
        return Ok(bars);
    };
    let mut anchor = first.close;
    let mut anchor_time = first.timestamp_micros;
    let mut direction = None;
    for (index, source) in source.iter().enumerate().skip(1) {
        let Some(box_size) = box_at(index).filter(|size| size.is_finite() && *size > 0.0) else {
            continue;
        };
        loop {
            let up_distance = source.close - anchor;
            let down_distance = anchor - source.close;
            let next = match direction {
                None if up_distance >= box_size => Some((Direction::Up, anchor, anchor + box_size)),
                None if down_distance >= box_size => {
                    Some((Direction::Down, anchor, anchor - box_size))
                }
                Some(Direction::Up) if up_distance >= box_size => {
                    Some((Direction::Up, anchor, anchor + box_size))
                }
                Some(Direction::Up) if down_distance >= box_size * 2.0 => {
                    Some((Direction::Down, anchor - box_size, anchor - box_size * 2.0))
                }
                Some(Direction::Down) if down_distance >= box_size => {
                    Some((Direction::Down, anchor, anchor - box_size))
                }
                Some(Direction::Down) if up_distance >= box_size * 2.0 => {
                    Some((Direction::Up, anchor + box_size, anchor + box_size * 2.0))
                }
                _ => None,
            };
            let Some((next_direction, open, close)) = next else {
                break;
            };
            output(&mut bars, anchor_time, source.timestamp_micros, open, close)?;
            anchor = close;
            anchor_time = source.timestamp_micros;
            direction = Some(next_direction);
        }
    }
    Ok(bars)
}

fn true_range(source: &[SyntheticSourceBar], index: usize) -> f64 {
    let bar = source[index];
    index.checked_sub(1).map_or(bar.high - bar.low, |prior| {
        let previous = source[prior].close;
        (bar.high - bar.low)
            .max((bar.high - previous).abs())
            .max((bar.low - previous).abs())
    })
}

fn atr_values(source: &[SyntheticSourceBar], period: usize) -> Vec<Option<f64>> {
    let mut atr = vec![None; source.len()];
    let mut ranges = Vec::with_capacity(source.len());
    for index in 0..source.len() {
        let range = true_range(source, index);
        ranges.push(range);
        if index + 1 == period {
            atr[index] = Some(ranges[..=index].iter().sum::<f64>() / period as f64);
        } else if index + 1 > period {
            let previous = atr[index - 1].unwrap_or(range);
            atr[index] = Some((previous * (period - 1) as f64 + range) / period as f64);
        }
    }
    atr
}

fn build_atr_renko_with_values(
    source: &[SyntheticSourceBar],
    atr: &[Option<f64>],
) -> Result<Vec<SyntheticBar>, SyntheticBarError> {
    let Some(first_ready) = atr.iter().position(Option::is_some) else {
        return Ok(Vec::new());
    };
    build_renko(&source[first_ready..], |index| atr[first_ready + index])
}

fn build_line_break(
    source: &[SyntheticSourceBar],
    lines: usize,
) -> Result<Vec<SyntheticBar>, SyntheticBarError> {
    let mut bars = Vec::new();
    for source in source {
        let Some(previous) = bars.last().copied() else {
            output(
                &mut bars,
                source.timestamp_micros,
                source.timestamp_micros,
                source.open,
                source.close,
            )?;
            continue;
        };
        let from = bars.len().saturating_sub(lines);
        let recent = &bars[from..];
        let high = recent
            .iter()
            .map(|bar| bar.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let low = recent
            .iter()
            .map(|bar| bar.low)
            .fold(f64::INFINITY, f64::min);
        if source.close > high || source.close < low {
            output(
                &mut bars,
                previous.close_timestamp_micros,
                source.timestamp_micros,
                previous.close,
                source.close,
            )?;
        }
    }
    Ok(bars)
}

fn build_kagi(
    source: &[SyntheticSourceBar],
    reversal_size: f64,
) -> Result<Vec<SyntheticBar>, SyntheticBarError> {
    let mut bars = Vec::new();
    let Some(first) = source.first() else {
        return Ok(bars);
    };
    let mut pivot = first.close;
    let mut pivot_time = first.timestamp_micros;
    let mut direction = None;
    for source in source.iter().skip(1) {
        match direction {
            None if source.close >= pivot + reversal_size => {
                output(
                    &mut bars,
                    pivot_time,
                    source.timestamp_micros,
                    pivot,
                    source.close,
                )?;
                pivot = source.close;
                direction = Some(Direction::Up);
            }
            None if source.close <= pivot - reversal_size => {
                output(
                    &mut bars,
                    pivot_time,
                    source.timestamp_micros,
                    pivot,
                    source.close,
                )?;
                pivot = source.close;
                direction = Some(Direction::Down);
            }
            Some(Direction::Up) if source.close > pivot => {
                if let Some(current) = bars.last_mut() {
                    current.close = source.close;
                    current.high = current.high.max(source.close);
                    current.close_timestamp_micros = source.timestamp_micros;
                }
                pivot = source.close;
            }
            Some(Direction::Up) if source.close <= pivot - reversal_size => {
                pivot_time = bars
                    .last()
                    .map_or(source.timestamp_micros, |bar| bar.close_timestamp_micros);
                output(
                    &mut bars,
                    pivot_time,
                    source.timestamp_micros,
                    pivot,
                    source.close,
                )?;
                pivot = source.close;
                direction = Some(Direction::Down);
            }
            Some(Direction::Down) if source.close < pivot => {
                if let Some(current) = bars.last_mut() {
                    current.close = source.close;
                    current.low = current.low.min(source.close);
                    current.close_timestamp_micros = source.timestamp_micros;
                }
                pivot = source.close;
            }
            Some(Direction::Down) if source.close >= pivot + reversal_size => {
                pivot_time = bars
                    .last()
                    .map_or(source.timestamp_micros, |bar| bar.close_timestamp_micros);
                output(
                    &mut bars,
                    pivot_time,
                    source.timestamp_micros,
                    pivot,
                    source.close,
                )?;
                pivot = source.close;
                direction = Some(Direction::Up);
            }
            _ => {}
        }
    }
    Ok(bars)
}

fn build_point_and_figure(
    source: &[SyntheticSourceBar],
    box_size: f64,
    reversal_boxes: u32,
) -> Result<Vec<SyntheticBar>, SyntheticBarError> {
    let mut bars = Vec::new();
    let Some(first) = source.first() else {
        return Ok(bars);
    };
    let mut pivot = first.close;
    let mut pivot_time = first.timestamp_micros;
    let mut direction = None;
    let reversal = box_size * f64::from(reversal_boxes);
    for source in source.iter().skip(1) {
        match direction {
            None if source.close >= pivot + box_size => {
                let boxes = ((source.close - pivot) / box_size).floor();
                let close = pivot + boxes * box_size;
                output(&mut bars, pivot_time, source.timestamp_micros, pivot, close)?;
                pivot = close;
                direction = Some(Direction::Up);
            }
            None if source.close <= pivot - box_size => {
                let boxes = ((pivot - source.close) / box_size).floor();
                let close = pivot - boxes * box_size;
                output(&mut bars, pivot_time, source.timestamp_micros, pivot, close)?;
                pivot = close;
                direction = Some(Direction::Down);
            }
            Some(Direction::Up) if source.close >= pivot + box_size => {
                let boxes = ((source.close - pivot) / box_size).floor();
                pivot += boxes * box_size;
                if let Some(current) = bars.last_mut() {
                    current.close = pivot;
                    current.high = pivot;
                    current.close_timestamp_micros = source.timestamp_micros;
                }
            }
            Some(Direction::Up) if source.close <= pivot - reversal => {
                let boxes = ((pivot - source.close) / box_size).floor();
                let close = pivot - boxes * box_size;
                pivot_time = bars
                    .last()
                    .map_or(source.timestamp_micros, |bar| bar.close_timestamp_micros);
                output(
                    &mut bars,
                    pivot_time,
                    source.timestamp_micros,
                    pivot - box_size,
                    close,
                )?;
                pivot = close;
                direction = Some(Direction::Down);
            }
            Some(Direction::Down) if source.close <= pivot - box_size => {
                let boxes = ((pivot - source.close) / box_size).floor();
                pivot -= boxes * box_size;
                if let Some(current) = bars.last_mut() {
                    current.close = pivot;
                    current.low = pivot;
                    current.close_timestamp_micros = source.timestamp_micros;
                }
            }
            Some(Direction::Down) if source.close >= pivot + reversal => {
                let boxes = ((source.close - pivot) / box_size).floor();
                let close = pivot + boxes * box_size;
                pivot_time = bars
                    .last()
                    .map_or(source.timestamp_micros, |bar| bar.close_timestamp_micros);
                output(
                    &mut bars,
                    pivot_time,
                    source.timestamp_micros,
                    pivot + box_size,
                    close,
                )?;
                pivot = close;
                direction = Some(Direction::Up);
            }
            _ => {}
        }
    }
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(values: &[f64]) -> Vec<SyntheticSourceBar> {
        values
            .iter()
            .enumerate()
            .map(|(index, &close)| SyntheticSourceBar {
                timestamp_micros: (index as i64 + 1) * 1_000_000,
                open: close,
                high: close,
                low: close,
                close,
            })
            .collect()
    }

    #[test]
    fn fixed_renko_requires_two_boxes_to_reverse_and_emits_every_crossed_box() {
        let mut bars =
            SyntheticBarAggregator::new(SyntheticBarOptions::RenkoFixed { box_size: 1.0 }).unwrap();
        bars.set_source(source(&[100.0, 103.2, 102.1, 100.8]))
            .unwrap();
        assert_eq!(
            bars.bars()
                .iter()
                .map(|bar| (bar.open, bar.close))
                .collect::<Vec<_>>(),
            vec![
                (100.0, 101.0),
                (101.0, 102.0),
                (102.0, 103.0),
                (102.0, 101.0)
            ]
        );
    }

    #[test]
    fn atr_renko_waits_for_wilder_warmup() {
        let mut bars =
            SyntheticBarAggregator::new(SyntheticBarOptions::RenkoAtr { period: 3 }).unwrap();
        let mut input = source(&[100.0, 101.0, 102.0, 105.0]);
        for bar in &mut input {
            bar.high += 0.5;
            bar.low -= 0.5;
        }
        bars.set_source(input).unwrap();
        assert_eq!(bars.bars().len(), 1);
        assert_eq!(bars.bars()[0].open_timestamp_micros, 3_000_000);
    }

    #[test]
    fn line_break_compares_close_against_last_n_line_extremes() {
        let mut bars =
            SyntheticBarAggregator::new(SyntheticBarOptions::LineBreak { lines: 3 }).unwrap();
        bars.set_source(source(&[100.0, 101.0, 102.0, 101.5, 99.0]))
            .unwrap();
        assert_eq!(
            bars.bars().iter().map(|bar| bar.close).collect::<Vec<_>>(),
            vec![100.0, 101.0, 102.0, 99.0]
        );
    }

    #[test]
    fn kagi_extends_direction_and_reverses_only_by_configured_amount() {
        let mut bars =
            SyntheticBarAggregator::new(SyntheticBarOptions::Kagi { reversal_size: 2.0 }).unwrap();
        bars.set_source(source(&[100.0, 103.0, 104.0, 103.0, 101.5, 99.0]))
            .unwrap();
        assert_eq!(
            bars.bars()
                .iter()
                .map(|bar| (bar.open, bar.close))
                .collect::<Vec<_>>(),
            vec![(100.0, 104.0), (104.0, 99.0)]
        );
    }

    #[test]
    fn point_and_figure_builds_columns_and_honors_reversal_boxes() {
        let mut bars = SyntheticBarAggregator::new(SyntheticBarOptions::PointAndFigure {
            box_size: 1.0,
            reversal_boxes: 3,
        })
        .unwrap();
        bars.set_source(source(&[100.0, 104.2, 102.0, 100.5, 103.8]))
            .unwrap();
        assert_eq!(
            bars.bars()
                .iter()
                .map(|bar| (bar.open, bar.close))
                .collect::<Vec<_>>(),
            vec![(100.0, 104.0), (103.0, 101.0)]
        );
    }

    #[test]
    fn source_replacement_is_atomic_and_ordered() {
        let mut bars =
            SyntheticBarAggregator::new(SyntheticBarOptions::LineBreak { lines: 3 }).unwrap();
        bars.set_source(source(&[100.0, 101.0])).unwrap();
        let before = bars.bars().to_vec();
        let mut invalid = source(&[102.0])[0];
        invalid.high = 100.0;
        assert!(bars.update_source(invalid).is_err());
        assert_eq!(bars.bars(), before);
    }

    #[test]
    fn ordered_tip_updates_match_full_rebuild_for_every_transform() {
        let input = source(&[100.0, 101.0, 103.0, 102.0, 98.0, 99.0, 104.0]);
        for options in [
            SyntheticBarOptions::RenkoFixed { box_size: 1.0 },
            SyntheticBarOptions::RenkoAtr { period: 3 },
            SyntheticBarOptions::LineBreak { lines: 3 },
            SyntheticBarOptions::Kagi { reversal_size: 2.0 },
            SyntheticBarOptions::PointAndFigure {
                box_size: 1.0,
                reversal_boxes: 3,
            },
        ] {
            let mut streamed = SyntheticBarAggregator::new(options).unwrap();
            for bar in &input {
                streamed.update_source(*bar).unwrap();
            }
            let mut rebuilt = SyntheticBarAggregator::new(options).unwrap();
            rebuilt.set_source(input.clone()).unwrap();
            assert_eq!(streamed.bars(), rebuilt.bars(), "{options:?}");
        }
    }

    #[test]
    fn chart_projection_owns_sequence_identity_replay_and_domain_exclusivity() {
        let mut chart = crate::ChartEngine::new(600.0, 400.0, 1.0);
        chart
            .configure_synthetic_bar_series(0, SyntheticBarOptions::RenkoFixed { box_size: 1.0 })
            .unwrap();
        chart
            .set_synthetic_bar_source(0, source(&[100.0, 103.0, 99.0]))
            .unwrap();
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 6);
        assert_eq!(chart.sequence_points().unwrap().len(), 6);

        chart.set_replay_clock_micros(Some(2_000_000)).unwrap();
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 3);
        assert_eq!(chart.sequence_points().unwrap().len(), 3);
        chart.set_replay_clock_micros(None).unwrap();
        assert_eq!(chart.data_layer().series_data(0).unwrap().0.len(), 6);

        let before = chart.synthetic_bars(0).unwrap().to_vec();
        chart
            .update_synthetic_bar_source(
                0,
                SyntheticSourceBar {
                    timestamp_micros: 4_000_000,
                    open: 99.0,
                    high: 105.0,
                    low: 99.0,
                    close: 105.0,
                },
            )
            .unwrap();
        assert_eq!(&chart.synthetic_bars(0).unwrap()[..before.len()], before);

        // The source owner is the only write path for synthetic projections: accepting generic
        // OHLC writes, pop, or an independent retention cap would desynchronize replay and the
        // logical sequence from the aggregator's canonical state.
        assert!(!chart.update_series_bar(0, 5.0, [1.0, 1.0, 1.0, 1.0]));
        assert!(chart
            .set_series_data(0, &[5.0], &[1.0], &[1.0], &[1.0], &[1.0])
            .is_err());
        assert_eq!(chart.series_pop(0, 1), None);
        assert!(!chart.set_series_max_points(0, Some(2)));

        let second = chart.add_series(crate::SeriesKind::Candlestick);
        assert_eq!(
            chart.configure_synthetic_bar_series(
                second,
                SyntheticBarOptions::LineBreak { lines: 3 },
            ),
            Err(SyntheticBarError::SequenceDomainInUse)
        );
        assert_eq!(
            chart.add_trade_stream(
                "second-sequence",
                crate::FootprintAggregationOptions {
                    bars: crate::FootprintBarAggregation::Trades { trades_per_bar: 10 },
                    ..crate::FootprintAggregationOptions::default()
                },
            ),
            Err(crate::FootprintError::SequenceDomainInUse)
        );
    }

    #[test]
    fn kagi_and_point_and_figure_emit_distinct_shared_frame_geometry() {
        let mut kagi = crate::ChartEngine::new(600.0, 400.0, 1.0);
        kagi.configure_synthetic_bar_series(0, SyntheticBarOptions::Kagi { reversal_size: 2.0 })
            .unwrap();
        kagi.set_synthetic_bar_source(0, source(&[100.0, 103.0, 104.0, 101.0]))
            .unwrap();
        kagi.time_scale.set_width(600.0);
        kagi.fit_content();
        kagi.set_visible_logical_range(-1.0, 2.0);
        kagi.build_frame();
        let frame = kagi.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            aeris_charts_render::draw_list::Prim::VLine { width, .. } if *width >= 2
        )));
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            aeris_charts_render::draw_list::Prim::HLine { width, .. } if *width >= 1
        )));

        let mut point_and_figure = crate::ChartEngine::new(600.0, 400.0, 1.0);
        point_and_figure
            .configure_synthetic_bar_series(
                0,
                SyntheticBarOptions::PointAndFigure {
                    box_size: 1.0,
                    reversal_boxes: 3,
                },
            )
            .unwrap();
        point_and_figure
            .set_synthetic_bar_source(0, source(&[100.0, 104.2, 100.5]))
            .unwrap();
        point_and_figure.time_scale.set_width(600.0);
        point_and_figure.fit_content();
        point_and_figure.set_visible_logical_range(-1.0, 2.0);
        point_and_figure.set_bar_spacing(20.0);
        point_and_figure.build_frame();
        let frame = point_and_figure.build_frame();
        let glyphs = frame.panes[0]
            .main
            .iter()
            .filter_map(|primitive| match primitive {
                aeris_charts_render::draw_list::Prim::Text { text, .. }
                    if text == "X" || text == "O" =>
                {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(glyphs.contains(&"X"));
        assert!(glyphs.contains(&"O"));
        assert!(glyphs.len() <= 4_096);
    }
}
