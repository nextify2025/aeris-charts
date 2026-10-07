//! Pure, confirmation-time structural studies over borrowed OHLC columns.

use crate::{
    IndicatorInput,
    study_annotations::{StudyAnnotations, StudyMarker, StudyMarkerKind, StudyZone},
};
use std::collections::VecDeque;

const CHECKPOINT_ROWS: usize = 1024;
pub const MAX_ORDER_BLOCK_SEARCH_ROWS: usize = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BreakOn {
    #[default]
    Close,
    Wick,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mitigation {
    #[default]
    Touch,
    Half,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MitigationPrice {
    #[default]
    Wick,
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrderBlockZone {
    #[default]
    Wick,
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StructureStudyKind {
    SwingPoints {
        left: usize,
        right: usize,
    },
    MarketStructure {
        left: usize,
        right: usize,
        break_on: BreakOn,
    },
    FairValueGaps {
        min_size: f64,
        mitigation: Mitigation,
        mitigation_price: MitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    },
    OrderBlocks {
        left: usize,
        right: usize,
        break_on: BreakOn,
        zone: OrderBlockZone,
        mitigation: Mitigation,
        mitigation_price: MitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    },
}

impl StructureStudyKind {
    pub fn swing_points(left: usize, right: usize) -> Self {
        Self::SwingPoints { left, right }
    }

    pub fn market_structure(left: usize, right: usize, break_on: BreakOn) -> Self {
        Self::MarketStructure {
            left,
            right,
            break_on,
        }
    }

    pub fn fair_value_gaps(
        min_size: f64,
        mitigation: Mitigation,
        mitigation_price: MitigationPrice,
        max_active: usize,
        show_mitigated: bool,
    ) -> Self {
        Self::FairValueGaps {
            min_size,
            mitigation,
            mitigation_price,
            max_active,
            show_mitigated,
        }
    }

    /// Validate before installing a study. Invalid parameters never silently change the rule.
    pub fn is_valid(self) -> bool {
        let valid_pivots = |left, right| (1..=50).contains(&left) && (1..=50).contains(&right);
        match self {
            Self::SwingPoints { left, right } | Self::MarketStructure { left, right, .. } => {
                valid_pivots(left, right)
            }
            Self::FairValueGaps {
                min_size,
                max_active,
                ..
            } => min_size.is_finite() && min_size >= 0. && (1..=64).contains(&max_active),
            Self::OrderBlocks {
                left,
                right,
                max_active,
                ..
            } => valid_pivots(left, right) && (1..=64).contains(&max_active),
        }
    }

    pub fn output_count(self) -> usize {
        if matches!(self, Self::SwingPoints { .. }) {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pivot {
    row: usize,
    confirm_row: usize,
    price: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScanState {
    high: Option<Pivot>,
    low: Option<Pivot>,
    high_broken: bool,
    low_broken: bool,
    trend: i8,
}

/// Only scanner state and active indices are checkpointed, never annotation history.
#[derive(Clone, Debug)]
struct ScanCheckpoint {
    state: ScanState,
    active: [VecDeque<usize>; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructureStudyResult {
    /// Swing highs and lows are stepped from confirmation; other kinds have one
    /// all-whitespace anchor column. All columns align to source rows.
    pub outputs: Vec<Vec<Option<f64>>>,
    pub annotations: StudyAnnotations,
}

/// Stateful append and historical repair. Sources are borrowed only for a call, never retained.
#[derive(Clone, Debug)]
pub struct StructureStudy {
    kind: StructureStudyKind,
    result: StructureStudyResult,
    state: ScanState,
    checkpoints: Vec<ScanCheckpoint>,
    len: usize,
    /// Rows the most recent [`Self::update`] scanned (work telemetry).
    last_work_rows: usize,
    #[cfg(test)]
    processed_rows: usize,
}

impl StructureStudy {
    pub fn new(kind: StructureStudyKind) -> Self {
        assert!(kind.is_valid(), "invalid structure study parameters");
        Self {
            kind,
            result: StructureStudyResult {
                outputs: vec![Vec::new(); kind.output_count()],
                annotations: StudyAnnotations::default(),
            },
            state: ScanState::default(),
            checkpoints: Vec::new(),
            len: 0,
            last_work_rows: 0,
            #[cfg(test)]
            processed_rows: 0,
        }
    }

    pub fn annotations(&self) -> &StudyAnnotations {
        &self.result.annotations
    }

    pub fn outputs(&self) -> &[Vec<Option<f64>>] {
        &self.result.outputs
    }

    pub fn result(&self) -> &StructureStudyResult {
        &self.result
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Rows the most recent [`Self::update`] scanned, including a checkpoint replay.
    pub fn last_work_rows(&self) -> usize {
        self.last_work_rows
    }

    pub fn capacity_bytes(&self) -> usize {
        self.result.annotations.capacity_bytes()
            + self.checkpoints.capacity() * std::mem::size_of::<ScanCheckpoint>()
            + self
                .checkpoints
                .iter()
                .map(|c| {
                    c.active
                        .iter()
                        .map(|a| a.capacity() * std::mem::size_of::<usize>())
                        .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .result
                .outputs
                .iter()
                .map(|v| v.capacity() * std::mem::size_of::<Option<f64>>())
                .sum::<usize>()
    }

    /// `from` is the earliest changed row, or the old length for an append.
    pub fn update(&mut self, input: IndicatorInput<'_>, from: usize) {
        let n = input
            .open
            .len()
            .min(input.high.len())
            .min(input.low.len())
            .min(input.close.len());
        assert!(
            from <= self.len && from <= n,
            "invalid structure repair row"
        );
        if from < self.len {
            let restart = from.min(n.saturating_sub(1)) / CHECKPOINT_ROWS * CHECKPOINT_ROWS;
            let checkpoint = if restart == 0 {
                ScanCheckpoint {
                    state: ScanState::default(),
                    active: [VecDeque::new(), VecDeque::new()],
                }
            } else {
                self.checkpoints[restart / CHECKPOINT_ROWS - 1].clone()
            };
            self.state = checkpoint.state;
            self.checkpoints.truncate(restart / CHECKPOINT_ROWS);
            self.result
                .annotations
                .rebuild_from_snapshot(restart, Some(checkpoint.active));
            for column in &mut self.result.outputs {
                column.truncate(restart);
            }
            self.process_rows(input, restart, n);
            self.last_work_rows = n - restart;
        } else {
            self.process_rows(input, from, n);
            self.last_work_rows = n - from;
        }
        self.len = n;
    }

    fn process_rows(&mut self, input: IndicatorInput<'_>, start: usize, end: usize) {
        for row in start..end {
            self.advance(input, row);
        }
    }

    fn advance(&mut self, input: IndicatorInput<'_>, row: usize) {
        #[cfg(test)]
        {
            self.processed_rows += 1;
        }
        for column in &mut self.result.outputs {
            column.push(None);
        }
        match self.kind {
            StructureStudyKind::FairValueGaps {
                min_size,
                mitigation,
                mitigation_price,
                max_active,
                ..
            } => {
                self.mitigate(input, row, mitigation, mitigation_price);
                if row >= 2 && valid(input, row) && valid(input, row - 1) && valid(input, row - 2) {
                    if input.low[row] - input.high[row - 2] > min_size.max(0.) {
                        self.add_zone(
                            StudyZone {
                                start_row: row - 1,
                                confirm_row: row,
                                top: input.low[row],
                                bottom: input.high[row - 2],
                                bullish: true,
                                end_row: None,
                                retired: false,
                            },
                            max_active,
                        );
                    }
                    if input.low[row - 2] - input.high[row] > min_size {
                        self.add_zone(
                            StudyZone {
                                start_row: row - 1,
                                confirm_row: row,
                                top: input.low[row - 2],
                                bottom: input.high[row],
                                bullish: false,
                                end_row: None,
                                retired: false,
                            },
                            max_active,
                        );
                    }
                }
            }
            kind => {
                if let StructureStudyKind::OrderBlocks {
                    mitigation,
                    mitigation_price,
                    ..
                } = kind
                {
                    self.mitigate(input, row, mitigation, mitigation_price);
                }
                let (left, right) = match kind {
                    StructureStudyKind::SwingPoints { left, right }
                    | StructureStudyKind::MarketStructure { left, right, .. }
                    | StructureStudyKind::OrderBlocks { left, right, .. } => (left, right),
                    StructureStudyKind::FairValueGaps { .. } => unreachable!(),
                };
                self.confirm_pivots(input, row, left, right);
                if matches!(kind, StructureStudyKind::SwingPoints { .. }) && valid(input, row) {
                    self.result.outputs[0][row] = self.state.high.map(|p| p.price);
                    self.result.outputs[1][row] = self.state.low.map(|p| p.price);
                } else if valid(input, row) {
                    self.break_structure(input, row, kind);
                }
            }
        }
        if (row + 1).is_multiple_of(CHECKPOINT_ROWS) {
            self.checkpoints.push(ScanCheckpoint {
                state: self.state,
                active: self.result.annotations.active_snapshot(),
            });
        }
    }

    fn confirm_pivots(&mut self, input: IndicatorInput<'_>, row: usize, left: usize, right: usize) {
        if row < left + right {
            return;
        }
        let center = row - right;
        if !(center - left..=row).all(|i| valid(input, i)) {
            return;
        }
        let high = (center - left..center).all(|i| input.high[center] > input.high[i])
            && (center + 1..=row).all(|i| input.high[center] >= input.high[i]);
        let low = (center - left..center).all(|i| input.low[center] < input.low[i])
            && (center + 1..=row).all(|i| input.low[center] <= input.low[i]);
        if high {
            self.state.high = Some(Pivot {
                row: center,
                confirm_row: row,
                price: input.high[center],
            });
            self.state.high_broken = false;
            if matches!(self.kind, StructureStudyKind::SwingPoints { .. }) {
                self.add_marker(StudyMarker {
                    row: center,
                    confirm_row: row,
                    price: input.high[center],
                    kind: StudyMarkerKind::SwingHigh,
                    from_row: None,
                });
            }
        }
        if low {
            self.state.low = Some(Pivot {
                row: center,
                confirm_row: row,
                price: input.low[center],
            });
            self.state.low_broken = false;
            if matches!(self.kind, StructureStudyKind::SwingPoints { .. }) {
                self.add_marker(StudyMarker {
                    row: center,
                    confirm_row: row,
                    price: input.low[center],
                    kind: StudyMarkerKind::SwingLow,
                    from_row: None,
                });
            }
        }
    }

    fn break_structure(&mut self, input: IndicatorInput<'_>, row: usize, kind: StructureStudyKind) {
        let break_on = match kind {
            StructureStudyKind::MarketStructure { break_on, .. }
            | StructureStudyKind::OrderBlocks { break_on, .. } => break_on,
            _ => return,
        };
        let up = self
            .state
            .high
            .filter(|p| !self.state.high_broken && p.confirm_row <= row)
            .filter(|p| {
                (if break_on == BreakOn::Wick {
                    input.high[row]
                } else {
                    input.close[row]
                }) > p.price
            });
        let down = self
            .state
            .low
            .filter(|p| !self.state.low_broken && p.confirm_row <= row)
            .filter(|p| {
                (if break_on == BreakOn::Wick {
                    input.low[row]
                } else {
                    input.close[row]
                }) < p.price
            });
        // An outside bar can cross both levels. Process up then down in
        // deterministic order; each confirmed swing still breaks only once.
        for (pivot, bullish) in [(up, true), (down, false)] {
            let Some(pivot) = pivot else { continue };
            let direction = if bullish { 1 } else { -1 };
            if bullish {
                self.state.high_broken = true;
            } else {
                self.state.low_broken = true;
            }
            let previous = self.state.trend;
            self.state.trend = direction;
            match kind {
                StructureStudyKind::MarketStructure { .. } => self.add_marker(StudyMarker {
                    row,
                    confirm_row: row,
                    price: pivot.price,
                    kind: if previous != 0 && previous != direction {
                        StudyMarkerKind::Choch { up: bullish }
                    } else {
                        StudyMarkerKind::Bos { up: bullish }
                    },
                    from_row: Some(pivot.row),
                }),
                StructureStudyKind::OrderBlocks {
                    zone, max_active, ..
                } => {
                    // The pivot row itself is eligible. Never look before p or
                    // further back than the last 500 preceding candles.
                    let start = pivot
                        .row
                        .max(row.saturating_sub(MAX_ORDER_BLOCK_SEARCH_ROWS));
                    if let Some(source) = (start..row).rev().find(|&i| {
                        valid(input, i)
                            && if bullish {
                                input.close[i] < input.open[i]
                            } else {
                                input.close[i] > input.open[i]
                            }
                    }) {
                        let top = if zone == OrderBlockZone::Wick {
                            input.high[source]
                        } else {
                            input.open[source].max(input.close[source])
                        };
                        let bottom = if zone == OrderBlockZone::Wick {
                            input.low[source]
                        } else {
                            input.open[source].min(input.close[source])
                        };
                        self.add_zone(
                            StudyZone {
                                start_row: source,
                                confirm_row: row,
                                top,
                                bottom,
                                bullish,
                                end_row: None,
                                retired: false,
                            },
                            max_active,
                        );
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    fn add_marker(&mut self, marker: StudyMarker) {
        self.result.annotations.push_marker(marker);
    }

    fn add_zone(&mut self, zone: StudyZone, max_active: usize) {
        self.result.annotations.push_zone_with_cap(zone, max_active);
    }

    fn mitigate(
        &mut self,
        input: IndicatorInput<'_>,
        row: usize,
        mitigation: Mitigation,
        price: MitigationPrice,
    ) {
        if !valid(input, row) {
            return;
        }
        let active: Vec<_> = self.result.annotations.active_indices().collect();
        for index in active {
            let z = self.result.annotations.zones()[index];
            if z.confirm_row >= row || z.end_row.is_some() {
                continue;
            }
            let threshold = match mitigation {
                Mitigation::Touch => {
                    if z.bullish {
                        z.top
                    } else {
                        z.bottom
                    }
                }
                Mitigation::Half => z.bottom + (z.top - z.bottom) / 2.,
                Mitigation::Full => {
                    if z.bullish {
                        z.bottom
                    } else {
                        z.top
                    }
                }
            };
            let crossed = if z.bullish {
                (if price == MitigationPrice::Wick {
                    input.low[row]
                } else {
                    input.close[row]
                }) <= threshold
            } else {
                (if price == MitigationPrice::Wick {
                    input.high[row]
                } else {
                    input.close[row]
                }) >= threshold
            };
            if crossed {
                self.result.annotations.end_zone(index, row);
            }
        }
    }
}

fn valid(input: IndicatorInput<'_>, row: usize) -> bool {
    let (o, h, l, c) = (
        input.open[row],
        input.high[row],
        input.low[row],
        input.close[row],
    );
    o.is_finite()
        && h.is_finite()
        && l.is_finite()
        && c.is_finite()
        && h >= o.max(c)
        && l <= o.min(c)
}

pub fn structure_study(
    input: IndicatorInput<'_>,
    kind: StructureStudyKind,
) -> StructureStudyResult {
    let mut study = StructureStudy::new(kind);
    study.update(input, 0);
    study.result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(o: &'a [f64], h: &'a [f64], l: &'a [f64], c: &'a [f64]) -> IndicatorInput<'a> {
        IndicatorInput {
            times: &[],
            open: o,
            high: h,
            low: l,
            close: c,
            volume: &[],
            amount: &[],
        }
    }

    fn fvg(mitigation: Mitigation, price: MitigationPrice, cap: usize) -> StructureStudyKind {
        StructureStudyKind::fair_value_gaps(0., mitigation, price, cap, true)
    }

    #[test]
    fn leftmost_plateau_and_unequal_left_right_confirmation() {
        let c = [4., 5., 8., 7., 7., 6., 6.];
        let h = [5., 6., 9., 9., 9., 7., 7.];
        let l = [3., 4., 5., 5., 5., 4., 4.];
        let out = structure_study(
            input(&c, &h, &l, &c),
            StructureStudyKind::swing_points(2, 2),
        );
        let highs: Vec<_> = out
            .annotations
            .markers()
            .iter()
            .filter(|m| m.kind == StudyMarkerKind::SwingHigh)
            .map(|m| (m.row, m.confirm_row))
            .collect();
        assert_eq!(highs, [(2, 4)]);
        assert_eq!(out.outputs[0][3], None);
        assert_eq!(out.outputs[0][4], Some(9.));
        assert_eq!(out.outputs[0][6], Some(9.));
        assert!(!StructureStudyKind::swing_points(0, 5).is_valid());
        assert!(!StructureStudyKind::swing_points(5, 51).is_valid());
        assert_eq!(StructureStudyKind::swing_points(5, 5).output_count(), 2);
    }

    #[test]
    fn close_and_wick_breaks_bos_choch_and_same_row_eligibility() {
        let o = [5., 7., 5., 7., 7., 5., 3.];
        let h = [6., 8., 6., 9., 8., 6., 4.];
        let l = [4., 6., 4., 6., 6., 4., 2.];
        let c = [5., 7., 5., 7., 7., 5., 3.];
        let wick = structure_study(
            input(&o, &h, &l, &c),
            StructureStudyKind::market_structure(1, 1, BreakOn::Wick),
        );
        assert!(wick.annotations.markers().iter().any(|m| {
            m.row == 3 && m.from_row == Some(1) && m.kind == StudyMarkerKind::Bos { up: true }
        }));
        assert!(
            wick.annotations
                .markers()
                .iter()
                .any(|m| { m.row == 6 && m.kind == StudyMarkerKind::Choch { up: false } })
        );
        let close = structure_study(
            input(&o, &h, &l, &c),
            StructureStudyKind::market_structure(1, 1, BreakOn::Close),
        );
        assert!(!close.annotations.markers().iter().any(|m| m.row == 3));
        assert!(close.outputs[0].iter().all(Option::is_none));
    }

    #[test]
    fn fair_value_gap_six_mitigation_combinations_and_active_cap() {
        let o = [9., 10., 13., 13., 12., 11.];
        let h = [10., 12., 15., 15., 14., 12.];
        let l = [8., 9., 13., 11., 9., 9.];
        let c = [9., 10., 14., 14., 11., 9.5];
        for (mode, price, end) in [
            (Mitigation::Touch, MitigationPrice::Wick, Some(3)),
            (Mitigation::Touch, MitigationPrice::Close, Some(4)),
            (Mitigation::Half, MitigationPrice::Wick, Some(3)),
            (Mitigation::Half, MitigationPrice::Close, Some(4)),
            (Mitigation::Full, MitigationPrice::Wick, Some(4)),
            (Mitigation::Full, MitigationPrice::Close, Some(5)),
        ] {
            let result = structure_study(input(&o, &h, &l, &c), fvg(mode, price, 20));
            let zone = result
                .annotations
                .zones()
                .iter()
                .find(|z| z.confirm_row == 2)
                .unwrap();
            assert_eq!(
                (zone.start_row, zone.bottom, zone.top, zone.end_row),
                (1, 10., 13., end)
            );
        }
        let h = [10., 10., 12., 12., 14., 14., 16.];
        let l = [8., 8., 11., 11., 13., 13., 15.];
        let c = [9., 9., 11.5, 11.5, 13.5, 13.5, 15.5];
        let result = structure_study(
            input(&c, &h, &l, &c),
            fvg(Mitigation::Full, MitigationPrice::Close, 1),
        );
        assert_eq!(
            result
                .annotations
                .zones()
                .iter()
                .filter(|z| z.bullish && z.end_row.is_none())
                .count(),
            1
        );
        assert_eq!(result.annotations.zones().back().unwrap().confirm_row, 6);
    }

    #[test]
    fn order_block_pivot_search_limit_zone_choice_and_absence() {
        let mut o = vec![5.; 504];
        let mut c = vec![5.; 504];
        let mut h = vec![6.; 504];
        let mut l = vec![4.; 504];
        o[0] = 5.5;
        h[1] = 8.;
        c[1] = 7.;
        c[503] = 9.;
        h[503] = 10.;
        let kind = |zone| StructureStudyKind::OrderBlocks {
            left: 1,
            right: 1,
            break_on: BreakOn::Close,
            zone,
            mitigation: Mitigation::Touch,
            mitigation_price: MitigationPrice::Wick,
            max_active: 20,
            show_mitigated: false,
        };
        assert!(
            structure_study(input(&o, &h, &l, &c), kind(OrderBlockZone::Wick))
                .annotations
                .zones()
                .is_empty()
        );
        // Exactly the earliest candidate within the 500-row window; pivot row
        // one is within range but not bearish.
        o[3] = 5.5;
        h[3] = 6.5;
        l[3] = 3.;
        let wick = structure_study(input(&o, &h, &l, &c), kind(OrderBlockZone::Wick));
        let body = structure_study(input(&o, &h, &l, &c), kind(OrderBlockZone::Body));
        assert_eq!(
            (
                wick.annotations.zones()[0].start_row,
                wick.annotations.zones()[0].top,
                wick.annotations.zones()[0].bottom
            ),
            (3, 6.5, 3.)
        );
        assert_eq!(
            (
                body.annotations.zones()[0].start_row,
                body.annotations.zones()[0].top,
                body.annotations.zones()[0].bottom
            ),
            (3, 5.5, 5.)
        );
        assert_eq!(wick.annotations.zones()[0].confirm_row, 503);
    }

    #[test]
    fn bearish_gap_and_block_ranges_are_symmetric() {
        let o = [10., 9., 6., 6.];
        let h = [12., 10., 7., 10.];
        let l = [9., 8., 5., 5.];
        let c = [10., 9., 6., 9.];
        let out = structure_study(
            input(&o, &h, &l, &c),
            fvg(Mitigation::Touch, MitigationPrice::Wick, 20),
        );
        assert_eq!(
            (
                out.annotations.zones()[0].start_row,
                out.annotations.zones()[0].top,
                out.annotations.zones()[0].bottom,
                out.annotations.zones()[0].end_row
            ),
            (1, 9., 7., Some(3))
        );
        let o = [8., 5., 8., 4., 4.];
        let h = [9., 6., 9., 5., 5.];
        let l = [7., 4., 7., 2., 3.];
        let c = [8., 5., 8.5, 3., 4.];
        let kind = StructureStudyKind::OrderBlocks {
            left: 1,
            right: 1,
            break_on: BreakOn::Close,
            zone: OrderBlockZone::Body,
            mitigation: Mitigation::Touch,
            mitigation_price: MitigationPrice::Close,
            max_active: 20,
            show_mitigated: true,
        };
        let blocks = structure_study(input(&o, &h, &l, &c), kind);
        assert_eq!(
            (
                blocks.annotations.zones()[0].start_row,
                blocks.annotations.zones()[0].top,
                blocks.annotations.zones()[0].bottom,
                blocks.annotations.zones()[0].bullish
            ),
            (2, 8.5, 8., false)
        );
    }

    #[test]
    fn capped_zone_repair_preserves_retired_history() {
        let n = 800;
        let o: Vec<_> = (0..n).map(|i| (i * 3) as f64).collect();
        let h: Vec<_> = o.iter().map(|v| v + 1.).collect();
        let mut l: Vec<_> = o.iter().map(|v| v - 1.).collect();
        let c = o.clone();
        let kind = fvg(Mitigation::Full, MitigationPrice::Close, 1);
        let mut study = StructureStudy::new(kind);
        study.update(input(&o, &h, &l, &c), 0);
        assert!(study.annotations().zones().len() > 64);
        assert!(study.annotations().zones()[0].retired);
        l[600] = o[598] + 1.;
        let before = study.processed_rows;
        study.update(input(&o, &h, &l, &c), 600);
        assert!(study.processed_rows - before <= n - 600 + CHECKPOINT_ROWS);
        assert_eq!(
            *study.result(),
            structure_study(input(&o, &h, &l, &c), kind)
        );
    }

    #[test]
    fn repeated_tip_replacement_preserves_all_markers() {
        // The tip is also the last row of a checkpoint block.
        let n = 8_192;
        let o = vec![100.; n];
        let mut h: Vec<_> = (0..n)
            .map(|i| if i.is_multiple_of(2) { 103. } else { 101. })
            .collect();
        let l: Vec<_> = (0..n)
            .map(|i| if i.is_multiple_of(2) { 99. } else { 97. })
            .collect();
        let mut study = StructureStudy::new(StructureStudyKind::swing_points(1, 1));
        study.update(input(&o, &h, &l, &o), 0);
        assert!(study.annotations().markers().len() > 4_096);
        for i in 0_usize..12 {
            h[n - 1] = if i.is_multiple_of(2) { 104. } else { 101. };
            let before = study.processed_rows;
            study.update(input(&o, &h, &l, &o), n - 1);
            assert!(study.processed_rows - before <= CHECKPOINT_ROWS);
            assert_eq!(
                *study.result(),
                structure_study(
                    input(&o, &h, &l, &o),
                    StructureStudyKind::swing_points(1, 1)
                )
            );
        }
    }

    #[test]
    fn repeated_tip_replacement_after_active_cap_restores_retired_zone() {
        let n = 4_200;
        let o: Vec<_> = (0..n).map(|i| (i * 3) as f64).collect();
        let h: Vec<_> = o.iter().map(|v| v + 1.).collect();
        let mut l: Vec<_> = o.iter().map(|v| v - 1.).collect();
        let kind = fvg(Mitigation::Full, MitigationPrice::Close, 1);
        let mut study = StructureStudy::new(kind);
        study.update(input(&o, &h, &l, &o), 0);
        assert!(study.annotations().zones()[0].retired);
        for i in 0_usize..12 {
            l[n - 1] = o[n - 1] - if i.is_multiple_of(2) { 1. } else { 5. };
            let before = study.processed_rows;
            study.update(input(&o, &h, &l, &o), n - 1);
            assert!(study.processed_rows - before <= CHECKPOINT_ROWS);
            assert_eq!(
                *study.result(),
                structure_study(input(&o, &h, &l, &o), kind)
            );
        }
    }

    #[test]
    fn long_history_repairs_all_four_studies_from_one_checkpoint_not_from_zero() {
        let n = 20_000;
        let mut o: Vec<_> = (0..n).map(|i| (i / 4 * 4) as f64 + 10.).collect();
        let mut h: Vec<_> = o.iter().map(|v| v + 2.).collect();
        let l: Vec<_> = o.iter().map(|v| v - 2.).collect();
        let mut c = o.clone();
        for i in 0..n {
            if i % 4 == 1 {
                h[i] += 3.;
                c[i] += 1.;
            }
            if i % 4 == 2 {
                o[i] += 1.;
                c[i] -= 1.;
            }
        }
        let kinds = [
            StructureStudyKind::swing_points(1, 1),
            StructureStudyKind::market_structure(1, 1, BreakOn::Wick),
            fvg(Mitigation::Full, MitigationPrice::Close, 2),
            StructureStudyKind::OrderBlocks {
                left: 1,
                right: 1,
                break_on: BreakOn::Wick,
                zone: OrderBlockZone::Body,
                mitigation: Mitigation::Full,
                mitigation_price: MitigationPrice::Close,
                max_active: 2,
                show_mitigated: true,
            },
        ];
        for kind in kinds {
            let mut study = StructureStudy::new(kind);
            study.update(input(&o, &h, &l, &c), 0);
            for annotations in [
                study
                    .annotations()
                    .markers()
                    .iter()
                    .map(|m| m.confirm_row)
                    .collect::<Vec<_>>(),
                study
                    .annotations()
                    .zones()
                    .iter()
                    .map(|z| z.confirm_row)
                    .collect::<Vec<_>>(),
            ] {
                assert!(
                    annotations.windows(3).all(|rows| rows[0] < rows[2]),
                    "{kind:?} emitted more than two annotations at one confirmation"
                );
            }
            for from in [17_123, 1_023, 1_024, 0] {
                c[from] += 0.25;
                let before = study.processed_rows;
                study.update(input(&o, &h, &l, &c), from);
                assert!(
                    study.processed_rows - before <= n - from + CHECKPOINT_ROWS,
                    "{kind:?} replayed {} rows",
                    study.processed_rows - before
                );
                assert_eq!(
                    *study.result(),
                    structure_study(input(&o, &h, &l, &c), kind),
                    "{kind:?}"
                );
            }
        }
    }

    #[test]
    fn prefix_append_tip_repair_and_checkpoint_match_batch() {
        let mut o = vec![5.; 620];
        let mut h = vec![6.; 620];
        let mut l = vec![4.; 620];
        let mut c = vec![5.; 620];
        for i in 0..620 {
            o[i] = 10. + (i % 9) as f64;
            c[i] = o[i] + if i % 4 == 0 { -0.5 } else { 0.5 };
            h[i] = o[i].max(c[i]) + 1.;
            l[i] = o[i].min(c[i]) - 1.;
        }
        let kinds = [
            StructureStudyKind::swing_points(3, 2),
            StructureStudyKind::market_structure(2, 3, BreakOn::Wick),
            fvg(Mitigation::Half, MitigationPrice::Wick, 20),
            StructureStudyKind::OrderBlocks {
                left: 2,
                right: 3,
                break_on: BreakOn::Close,
                zone: OrderBlockZone::Body,
                mitigation: Mitigation::Touch,
                mitigation_price: MitigationPrice::Close,
                max_active: 20,
                show_mitigated: true,
            },
        ];
        for kind in kinds {
            let mut study = StructureStudy::new(kind);
            for n in 1..=620 {
                study.update(input(&o[..n], &h[..n], &l[..n], &c[..n]), n - 1);
            }
            assert_eq!(
                *study.result(),
                structure_study(input(&o, &h, &l, &c), kind)
            );
            for index in [0, 255, 256, 440, 619] {
                c[index] = o[index];
                study.update(input(&o, &h, &l, &c), index);
                assert_eq!(
                    *study.result(),
                    structure_study(input(&o, &h, &l, &c), kind)
                );
                c[index] = o[index] + if index % 4 == 0 { -0.5 } else { 0.5 };
                study.update(input(&o, &h, &l, &c), index);
            }
        }
    }
}
