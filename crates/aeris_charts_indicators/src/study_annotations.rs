//! Chart-independent annotation output bounded by retained source rows.

use std::collections::VecDeque;

pub const MAX_ACTIVE_ZONES_PER_SIDE: usize = 64;

/// Host-supplied UTC interval: inclusive start, exclusive end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSpan {
    pub start: i64,
    pub end: i64,
    pub session_id: u64,
}

impl SessionSpan {
    /// UTC civil-day ordinal of the last included second, never a host-local date.
    pub fn trading_day(self) -> i64 {
        (self.end - 1).div_euclid(86_400)
    }
}

#[cfg(test)]
mod session_tests {
    use super::SessionSpan;

    #[test]
    fn host_trading_date_uses_the_last_included_utc_second() {
        let span = SessionSpan {
            start: 86_399,
            end: 86_401,
            session_id: 7,
        };
        assert_eq!(span.trading_day(), 1);
        let ending_at_midnight = SessionSpan {
            end: 86_400,
            ..span
        };
        assert_eq!(ending_at_midnight.trading_day(), 0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyMarkerKind {
    SwingHigh,
    SwingLow,
    Bos { up: bool },
    Choch { up: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StudyMarker {
    pub row: usize,
    pub confirm_row: usize,
    pub price: f64,
    pub kind: StudyMarkerKind,
    /// Pivot row for a BOS/CHoCH segment.
    pub from_row: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StudyZone {
    pub start_row: usize,
    pub confirm_row: usize,
    pub top: f64,
    pub bottom: f64,
    pub bullish: bool,
    /// First row that mitigates the zone, if any.
    pub end_row: Option<usize>,
    /// Retired by the active-zone limit, rather than mitigated by price.
    pub retired: bool,
}

/// Confirmation-ordered history. Swing and structure emit at most two markers per
/// confirmation row; FVG and order blocks emit at most two zones per row.
/// History is removed only when its source rows are removed.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct StudyAnnotations {
    markers: VecDeque<StudyMarker>,
    zones: VecDeque<StudyZone>,
    #[serde(skip)]
    marker_index: RowIntervalIndex,
    #[serde(skip)]
    closed_zone_index: RowIntervalIndex,
    #[serde(skip)]
    active_zones: [VecDeque<usize>; 2],
    #[serde(skip)]
    ends: Vec<(usize, usize)>,
}

/// Confirmation-order segment tree. Each node bounds the rows in its subtree;
/// a viewport query visits only branches whose intervals can intersect it.
/// Leaves for active zones are empty; active zones have a separate capped list.
#[derive(Clone, Debug, Default)]
struct RowIntervalIndex {
    tree: Vec<(usize, usize)>,
    base: usize,
    len: usize,
    #[cfg(test)]
    repair_nodes: usize,
}

impl RowIntervalIndex {
    const EMPTY: (usize, usize) = (usize::MAX, 0);

    fn merge(a: (usize, usize), b: (usize, usize)) -> (usize, usize) {
        (a.0.min(b.0), a.1.max(b.1))
    }

    fn set(&mut self, index: usize, value: (usize, usize)) {
        if index >= self.base {
            let old = std::mem::take(&mut self.tree);
            let old_base = self.base;
            self.base = (index + 1).next_power_of_two();
            self.tree = vec![Self::EMPTY; self.base * 2];
            if old_base != 0 {
                self.tree[self.base..self.base + self.len]
                    .copy_from_slice(&old[old_base..old_base + self.len]);
            }
            for node in (1..self.base).rev() {
                self.tree[node] = Self::merge(self.tree[2 * node], self.tree[2 * node + 1]);
            }
        }
        self.len = self.len.max(index + 1);
        let mut node = self.base + index;
        self.tree[node] = value;
        #[cfg(test)]
        {
            self.repair_nodes += 1;
        }
        while node > 1 {
            node /= 2;
            self.tree[node] = Self::merge(self.tree[2 * node], self.tree[2 * node + 1]);
            #[cfg(test)]
            {
                self.repair_nodes += 1;
            }
        }
    }

    /// Clear discarded leaves and recompute only their ancestors. The tree's
    /// base and allocation survive suffix repair; each level covers at most
    /// half as many nodes as the one below it.
    fn truncate(&mut self, len: usize) {
        assert!(len <= self.len);
        if len == self.len {
            return;
        }
        let (mut first, mut end) = (self.base + len, self.base + self.len);
        self.tree[first..end].fill(Self::EMPTY);
        #[cfg(test)]
        {
            self.repair_nodes += end - first;
        }
        self.len = len;
        while first > 1 {
            first /= 2;
            end = end.div_ceil(2);
            for node in first..end {
                self.tree[node] = Self::merge(self.tree[2 * node], self.tree[2 * node + 1]);
                #[cfg(test)]
                {
                    self.repair_nodes += 1;
                }
            }
        }
    }

    fn visit(&self, from: usize, to: usize, mut visit: impl FnMut(usize)) {
        if self.len == 0 || from >= to {
            return;
        }
        self.visit_node(1, 0, self.base, from, to, &mut visit);
    }

    fn visit_node(
        &self,
        node: usize,
        start: usize,
        end: usize,
        from: usize,
        to: usize,
        visit: &mut impl FnMut(usize),
    ) {
        let (min_start, max_end) = self.tree[node];
        if min_start >= to || max_end < from || min_start == usize::MAX {
            return;
        }
        if end - start == 1 {
            if start < self.len {
                visit(start);
            }
            return;
        }
        let mid = start + (end - start) / 2;
        self.visit_node(node * 2, start, mid, from, to, visit);
        self.visit_node(node * 2 + 1, mid, end, from, to, visit);
    }

    fn capacity_bytes(&self) -> usize {
        self.tree.capacity() * std::mem::size_of::<(usize, usize)>()
    }
}

// Query accelerators are derived storage, not observable annotation state. In
// particular, append-then-repair may leave a larger tree than a batch build.
impl PartialEq for StudyAnnotations {
    fn eq(&self, other: &Self) -> bool {
        self.markers == other.markers
            && self.zones == other.zones
            && self.active_zones == other.active_zones
            && self.ends == other.ends
    }
}

impl StudyAnnotations {
    fn side(bullish: bool) -> usize {
        usize::from(bullish)
    }

    fn marker_interval(marker: StudyMarker) -> (usize, usize) {
        match marker.kind {
            StudyMarkerKind::Bos { .. } | StudyMarkerKind::Choch { .. } => {
                marker.from_row.map_or((marker.row, marker.row), |start| {
                    (start.min(marker.row), start.max(marker.row))
                })
            }
            StudyMarkerKind::SwingHigh | StudyMarkerKind::SwingLow => (marker.row, marker.row),
        }
    }

    pub fn markers(&self) -> &VecDeque<StudyMarker> {
        &self.markers
    }

    pub fn zones(&self) -> &VecDeque<StudyZone> {
        &self.zones
    }

    pub fn capacity_bytes(&self) -> usize {
        self.markers.capacity() * std::mem::size_of::<StudyMarker>()
            + self.zones.capacity() * std::mem::size_of::<StudyZone>()
            + self.ends.capacity() * std::mem::size_of::<(usize, usize)>()
            + self.marker_index.capacity_bytes()
            + self.closed_zone_index.capacity_bytes()
            + self
                .active_zones
                .iter()
                .map(|side| side.capacity() * std::mem::size_of::<usize>())
                .sum::<usize>()
    }

    pub(crate) fn active_snapshot(&self) -> [VecDeque<usize>; 2] {
        self.active_zones.clone()
    }

    pub fn push_marker(&mut self, marker: StudyMarker) {
        assert!(
            self.markers
                .back()
                .is_none_or(|last| last.confirm_row <= marker.confirm_row),
            "study markers must be appended in confirmation order"
        );
        self.markers.push_back(marker);
        let index = self.markers.len() - 1;
        self.marker_index.set(index, Self::marker_interval(marker));
    }

    pub fn push_zone(&mut self, zone: StudyZone) {
        self.push_zone_with_cap(zone, MAX_ACTIVE_ZONES_PER_SIDE);
    }

    pub fn push_zone_with_cap(&mut self, zone: StudyZone, cap: usize) {
        assert!((1..=MAX_ACTIVE_ZONES_PER_SIDE).contains(&cap));
        assert!(
            self.zones
                .back()
                .is_none_or(|last| last.confirm_row <= zone.confirm_row),
            "study zones must be appended in confirmation order"
        );
        let side = Self::side(zone.bullish);
        if zone.end_row.is_none() {
            if self.active_zones[side].len() == cap {
                let oldest = self.active_zones[side].pop_front().expect("active zone");
                self.zones[oldest].end_row = Some(zone.confirm_row);
                self.zones[oldest].retired = true;
                self.ends.push((zone.confirm_row, oldest));
                self.closed_zone_index
                    .set(oldest, (self.zones[oldest].start_row, zone.confirm_row));
            }
            self.active_zones[side].push_back(self.zones.len());
        }
        self.zones.push_back(zone);
        let index = self.zones.len() - 1;
        self.closed_zone_index.set(
            index,
            zone.end_row
                .map_or(RowIntervalIndex::EMPTY, |end| (zone.start_row, end)),
        );
    }

    /// Record mitigation for a retained zone by its current deque index.
    /// Returns false for an unknown index or a mitigation preceding confirmation.
    pub fn end_zone(&mut self, index: usize, end_row: usize) -> bool {
        let Some(zone) = self.zones.get_mut(index) else {
            return false;
        };
        if end_row < zone.confirm_row || zone.end_row.is_some() {
            return false;
        }
        self.active_zones[Self::side(zone.bullish)].retain(|&active| active != index);
        zone.end_row = Some(end_row);
        self.ends.push((end_row, index));
        self.closed_zone_index.set(index, (zone.start_row, end_row));
        true
    }

    /// Discard confirmations in the repaired suffix and undo its mitigations.
    ///
    /// Confirmation order makes the retained prefix searchable without scanning the history.
    pub fn rebuild_from(&mut self, from: usize) {
        self.rebuild_from_snapshot(from, None);
    }

    /// Restore a small checkpoint's active indices without inspecting old history.
    pub(crate) fn rebuild_from_snapshot(
        &mut self,
        from: usize,
        active: Option<[VecDeque<usize>; 2]>,
    ) {
        fn first_at_or_after<T>(
            items: &VecDeque<T>,
            from: usize,
            row: impl Fn(&T) -> usize,
        ) -> usize {
            let (mut lo, mut hi) = (0, items.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if row(&items[mid]) < from {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        }
        let marker_end = first_at_or_after(&self.markers, from, |m| m.confirm_row);
        self.marker_index.truncate(marker_end);
        self.markers.truncate(marker_end);
        let zone_end = first_at_or_after(&self.zones, from, |z| z.confirm_row);
        self.closed_zone_index.truncate(zone_end);
        self.zones.truncate(zone_end);
        while self.ends.last().is_some_and(|&(end, _)| end >= from) {
            let (_, index) = self.ends.pop().expect("last end");
            if index < self.zones.len() {
                self.zones[index].end_row = None;
                self.zones[index].retired = false;
                self.closed_zone_index.set(index, RowIntervalIndex::EMPTY);
            }
        }
        self.active_zones = active.unwrap_or_else(|| {
            let mut sides = [VecDeque::new(), VecDeque::new()];
            for (i, z) in self.zones.iter().enumerate() {
                if z.end_row.is_none() {
                    sides[Self::side(z.bullish)].push_back(i);
                }
            }
            sides
        });
    }

    /// Source-row viewport `[from, to)`, preserving confirmation paint order.
    /// The count includes only candidates handed to the painter.
    pub fn visit_visible_zones(
        &self,
        from: usize,
        to: usize,
        show_mitigated: bool,
        mut visit: impl FnMut(&StudyZone),
    ) {
        if from >= to {
            return;
        }
        // Two capped sides; avoid a per-frame heap allocation.
        let mut active = [0; MAX_ACTIVE_ZONES_PER_SIDE * 2];
        let mut count = 0;
        for i in self.active_indices() {
            active[count] = i;
            count += 1;
        }
        let active = &mut active[..count];
        active.sort_unstable();
        if show_mitigated {
            // Merge the capped active list with closed leaves in confirmation order.
            let mut next = 0;
            self.closed_zone_index.visit(from, to, |i| {
                while next < active.len() && active[next] < i {
                    let z = &self.zones[active[next]];
                    if z.start_row < to {
                        visit(z);
                    }
                    next += 1;
                }
                visit(&self.zones[i]);
            });
            for &i in &active[next..] {
                let z = &self.zones[i];
                if z.start_row < to {
                    visit(z);
                }
            }
        } else {
            for &i in active.iter() {
                let z = &self.zones[i];
                if z.start_row < to {
                    visit(z);
                }
            }
        }
    }

    /// Source-row viewport `[from, to)`, in confirmation paint order. Structure
    /// strokes intersecting the window are returned even if their break row is offscreen.
    pub fn visit_visible_markers(
        &self,
        from: usize,
        to: usize,
        mut visit: impl FnMut(&StudyMarker),
    ) {
        self.marker_index
            .visit(from, to, |i| visit(&self.markers[i]));
    }

    pub(crate) fn active_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.active_zones
            .iter()
            .flat_map(|side| side.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(confirm_row: usize) -> StudyMarker {
        StudyMarker {
            row: confirm_row.saturating_sub(2),
            confirm_row,
            price: 100.0,
            kind: StudyMarkerKind::SwingHigh,
            from_row: None,
        }
    }

    fn zone(confirm_row: usize, bullish: bool) -> StudyZone {
        StudyZone {
            start_row: confirm_row.saturating_sub(1),
            confirm_row,
            top: 101.0,
            bottom: 100.0,
            bullish,
            end_row: None,
            retired: false,
        }
    }

    #[test]
    fn long_history_retains_every_annotation_until_source_rebuild() {
        let mut annotations = StudyAnnotations::default();
        for row in 0..10_000 {
            annotations.push_marker(marker(row));
            let mut finished = zone(row, row % 2 == 0);
            finished.end_row = Some(row);
            annotations.push_zone(finished);
        }
        assert_eq!(annotations.markers().len(), 10_000);
        assert_eq!(annotations.zones().len(), 10_000);
        assert!(
            annotations.capacity_bytes()
                >= 10_000 * (std::mem::size_of::<StudyMarker>() + std::mem::size_of::<StudyZone>())
        );
        annotations.rebuild_from(5_000);
        assert_eq!(annotations.markers().len(), 5_000);
        assert_eq!(annotations.zones().len(), 5_000);
        assert_eq!(annotations.marker_index.len, 5_000);
        assert_eq!(annotations.closed_zone_index.len, 5_000);
    }

    #[test]
    fn over_cap_retires_oldest_without_deleting_history() {
        let mut annotations = StudyAnnotations::default();
        for row in 0..10_000 {
            annotations.push_zone_with_cap(zone(row, true), 1);
        }
        assert_eq!(annotations.zones().len(), 10_000);
        assert_eq!(annotations.zones()[0].end_row, Some(1));
        assert!(annotations.zones()[0].retired);
        assert_eq!(annotations.active_indices().count(), 1);
        annotations.rebuild_from(9_999);
        assert_eq!(annotations.zones().len(), 9_999);
        assert!(!annotations.zones()[9_998].retired);
        assert_eq!(annotations.active_indices().count(), 1);
    }

    #[test]
    fn repair_truncates_confirmations_and_clears_suffix_mitigation() {
        let mut annotations = StudyAnnotations::default();
        for row in [2, 4, 4, 6] {
            annotations.push_marker(marker(row));
            annotations.push_zone(zone(row, true));
        }
        assert!(!annotations.end_zone(0, 1));
        assert!(annotations.end_zone(0, 3));
        assert!(!annotations.end_zone(0, 5));
        assert!(annotations.end_zone(1, 5));
        assert!(!annotations.end_zone(4, 7));
        annotations.rebuild_from(5);
        assert_eq!(
            annotations
                .markers()
                .iter()
                .map(|m| m.confirm_row)
                .collect::<Vec<_>>(),
            [2, 4, 4]
        );
        assert_eq!(annotations.zones().len(), 3);
        assert_eq!(annotations.zones()[0].end_row, Some(3));
        assert_eq!(annotations.zones()[1].end_row, None);
        annotations.rebuild_from(4);
        assert_eq!(annotations.markers().len(), 1);
        assert_eq!(annotations.zones().len(), 1);
    }

    #[test]
    fn repair_reactivation_preserves_per_side_cap() {
        let mut annotations = StudyAnnotations::default();
        for row in 0..MAX_ACTIVE_ZONES_PER_SIDE {
            annotations.push_zone(zone(row, true));
            assert!(annotations.end_zone(row, 100));
        }
        annotations.rebuild_from(100);
        assert_eq!(annotations.zones().len(), MAX_ACTIVE_ZONES_PER_SIDE);
        assert_eq!(annotations.zones()[0].confirm_row, 0);
        assert!(annotations.zones().iter().all(|z| z.end_row.is_none()));
        annotations.push_zone(zone(101, true));
        assert_eq!(annotations.zones().len(), MAX_ACTIVE_ZONES_PER_SIDE + 1);
        assert!(annotations.zones()[0].retired);
    }

    #[test]
    fn long_history_frame_query_visits_only_intersecting_rows() {
        let mut annotations = StudyAnnotations::default();
        for row in 0..100_000 {
            annotations.push_marker(marker(row));
            let mut finished = zone(row, row % 2 == 0);
            finished.end_row = Some(row);
            annotations.push_zone(finished);
        }
        annotations.push_marker(StudyMarker {
            row: 100_000,
            confirm_row: 100_000,
            price: 100.0,
            kind: StudyMarkerKind::Bos { up: true },
            from_row: Some(1),
        });
        // An old, long closed interval must survive even when its confirmation
        // is far outside the viewport and start rows are not monotonic.
        let mut spanning = zone(100_000, true);
        spanning.start_row = 1;
        spanning.end_row = Some(100_001);
        annotations.push_zone(spanning);
        annotations.push_zone(zone(100_002, false));

        let mut marker_rows = Vec::new();
        annotations.visit_visible_markers(80_000, 80_010, |m| marker_rows.push(m.confirm_row));
        assert_eq!(
            marker_rows,
            (80_002..80_012)
                .chain(std::iter::once(100_000))
                .collect::<Vec<_>>()
        );
        let mut zone_rows = Vec::new();
        annotations.visit_visible_zones(80_000, 80_010, true, |z| zone_rows.push(z.confirm_row));
        assert_eq!(
            zone_rows,
            (80_000..80_011)
                .chain(std::iter::once(100_000))
                .collect::<Vec<_>>()
        );
        zone_rows.clear();
        annotations.visit_visible_zones(80_000, 80_010, false, |z| zone_rows.push(z.confirm_row));
        assert!(zone_rows.is_empty());
        assert!(annotations.capacity_bytes() > 100_000 * std::mem::size_of::<StudyZone>());

        annotations.rebuild_from(90_000);
        marker_rows.clear();
        annotations.visit_visible_markers(80_000, 80_010, |m| marker_rows.push(m.confirm_row));
        assert_eq!(marker_rows, (80_002..80_012).collect::<Vec<_>>());
        zone_rows.clear();
        annotations.visit_visible_zones(80_000, 80_010, true, |z| zone_rows.push(z.confirm_row));
        assert_eq!(zone_rows, (80_000..80_011).collect::<Vec<_>>());
        annotations.rebuild_from(80_005);
        zone_rows.clear();
        annotations.visit_visible_zones(80_000, 80_010, true, |z| zone_rows.push(z.confirm_row));
        assert_eq!(zone_rows, (80_000..80_005).collect::<Vec<_>>());
    }

    #[test]
    fn mitigation_and_retirement_update_visible_intervals() {
        let mut annotations = StudyAnnotations::default();
        annotations.push_zone_with_cap(zone(2, true), 1);
        annotations.push_zone_with_cap(zone(10, true), 1);
        let mut rows = Vec::new();
        annotations.visit_visible_zones(3, 4, true, |z| rows.push(z.confirm_row));
        assert_eq!(rows, [2]);
        rows.clear();
        annotations.visit_visible_zones(11, 12, true, |z| rows.push(z.confirm_row));
        assert_eq!(rows, [10]);
        rows.clear();
        assert!(annotations.end_zone(1, 12));
        annotations.visit_visible_zones(11, 12, true, |z| rows.push(z.confirm_row));
        assert_eq!(rows, [10]);
        annotations.rebuild_from(12);
        rows.clear();
        annotations.visit_visible_zones(11, 12, false, |z| rows.push(z.confirm_row));
        assert_eq!(rows, [10]);
    }

    #[test]
    fn repair_and_batch_history_compare_equal_despite_reused_index_capacity() {
        let mut batch = StudyAnnotations::default();
        let mut repaired = StudyAnnotations::default();
        for row in 0..40 {
            if row < 20 {
                batch.push_marker(marker(row));
                let mut finished = zone(row, true);
                finished.end_row = Some(row);
                batch.push_zone(finished);
            }
            repaired.push_marker(marker(row));
            let mut finished = zone(row, true);
            finished.end_row = Some(row);
            repaired.push_zone(finished);
        }
        repaired.rebuild_from(20);
        assert_eq!(batch, repaired);
        for from in [0, 5, 19, 20, 30] {
            let visible = |annotations: &StudyAnnotations| {
                let mut markers = Vec::new();
                let mut zones = Vec::new();
                annotations.visit_visible_markers(from, from + 2, |m| markers.push(m.confirm_row));
                annotations
                    .visit_visible_zones(from, from + 2, true, |z| zones.push(z.confirm_row));
                (markers, zones)
            };
            assert_eq!(visible(&batch), visible(&repaired));
        }
    }

    #[test]
    fn long_history_tip_repair_reuses_both_trees_with_bounded_index_work() {
        let mut annotations = StudyAnnotations::default();
        for row in 0..100_000 {
            annotations.push_marker(marker(row));
            let mut finished = zone(row, true);
            if row != 123 {
                finished.end_row = Some(row);
            }
            annotations.push_zone(finished);
        }
        assert!(annotations.end_zone(123, 99_999));
        let marker_tree = annotations.marker_index.tree.as_ptr();
        let zone_tree = annotations.closed_zone_index.tree.as_ptr();
        let marker_capacity = annotations.marker_index.tree.capacity();
        let zone_capacity = annotations.closed_zone_index.tree.capacity();
        for _ in 0..8 {
            annotations.marker_index.repair_nodes = 0;
            annotations.closed_zone_index.repair_nodes = 0;
            annotations.rebuild_from(99_999);
            assert_eq!(annotations.zones()[123].end_row, None);
            annotations.push_marker(marker(99_999));
            let mut finished = zone(99_999, true);
            finished.end_row = Some(99_999);
            annotations.push_zone(finished);
            assert!(annotations.end_zone(123, 99_999));
            assert_eq!(annotations.marker_index.tree.as_ptr(), marker_tree);
            assert_eq!(annotations.closed_zone_index.tree.as_ptr(), zone_tree);
            assert_eq!(annotations.marker_index.tree.capacity(), marker_capacity);
            assert_eq!(annotations.closed_zone_index.tree.capacity(), zone_capacity);
            assert!(annotations.marker_index.repair_nodes < 128);
            assert!(annotations.closed_zone_index.repair_nodes < 128);
        }
    }

    #[test]
    #[should_panic(expected = "study markers must be appended in confirmation order")]
    fn marker_out_of_order_is_rejected() {
        let mut annotations = StudyAnnotations::default();
        annotations.push_marker(marker(2));
        annotations.push_marker(marker(1));
    }

    #[test]
    #[should_panic(expected = "study zones must be appended in confirmation order")]
    fn zone_out_of_order_is_rejected() {
        let mut annotations = StudyAnnotations::default();
        annotations.push_zone(zone(2, true));
        annotations.push_zone(zone(1, false));
    }
}
