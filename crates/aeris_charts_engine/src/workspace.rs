//! Multi-chart workspace: the split-grid MODEL behind the platform's chart-splitting feature.
//!
//! A workspace is a binary tree of cells. Every leaf cell is an independent chart (its own
//! engine instance, scales, series, chart type, and drawing primitives on the host side) —
//! splitting never shares or mirrors state between cells. A cell splits horizontally (side by
//! side) or vertically (stacked), recursively and without a built-in cap; removing a cell
//! collapses its split node so the sibling subtree absorbs the freed space.
//!
//! Subscription limits, billing meters, and cell-age policy are host-owned. This engine type is
//! only the generic, rendering-agnostic layout primitive.

/// Split orientation: `Horizontal` places the two charts side by side, `Vertical` stacks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

/// Immutable snapshot of a workspace tree: either a chart cell or a split of two subtrees.
/// The split's `ratio` is the `a` subtree's share of the space (0..1, default 0.5); dragging
/// the divider between two adjacent cells adjusts it via [`Workspace::resize_between`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkspaceLayout {
    /// A leaf containing one chart cell.
    Cell {
        /// Stable workspace cell identifier.
        id: u64,
    },
    /// A directional split containing two child layouts.
    Split {
        /// Direction in which the child layouts are arranged.
        direction: SplitDirection,
        /// Fraction of the available space assigned to `a`.
        ratio: f64,
        /// First child subtree.
        a: Box<WorkspaceLayout>,
        /// Second child subtree.
        b: Box<WorkspaceLayout>,
    },
}

impl WorkspaceLayout {
    fn cell_ids(&self, out: &mut Vec<u64>) {
        match self {
            WorkspaceLayout::Cell { id } => out.push(*id),
            WorkspaceLayout::Split { a, b, .. } => {
                a.cell_ids(out);
                b.cell_ids(out);
            }
        }
    }

    fn first_leaf(&self) -> u64 {
        match self {
            WorkspaceLayout::Cell { id } => *id,
            WorkspaceLayout::Split { a, .. } => a.first_leaf(),
        }
    }

    fn last_leaf(&self) -> u64 {
        match self {
            WorkspaceLayout::Cell { id } => *id,
            WorkspaceLayout::Split { b, .. } => b.last_leaf(),
        }
    }

    /// Stable leaf identities in visual traversal order.
    #[must_use]
    pub fn leaf_ids(&self) -> Vec<u64> {
        let mut ids = Vec::new();
        self.cell_ids(&mut ids);
        ids
    }

    /// Ratio owned by the split at one adjacent leaf boundary.
    #[must_use]
    pub fn boundary_ratio(&self, left_id: u64, right_id: u64) -> Option<f64> {
        match self {
            Self::Cell { .. } => None,
            Self::Split { ratio, a, b, .. } => {
                if a.last_leaf() == left_id && b.first_leaf() == right_id {
                    Some(*ratio)
                } else {
                    a.boundary_ratio(left_id, right_id)
                        .or_else(|| b.boundary_ratio(left_id, right_id))
                }
            }
        }
    }

    /// Normalized 10,000-point leaf weights for legacy persistence migration.
    #[must_use]
    pub fn basis_points(&self) -> Vec<(u64, u32)> {
        let mut weighted = Vec::new();
        collect_weights(self, 1.0, &mut weighted);
        let mut remaining = 10_000_u32;
        let last = weighted.len().saturating_sub(1);
        weighted
            .into_iter()
            .enumerate()
            .map(|(index, (id, weight))| {
                let basis = if index == last {
                    remaining
                } else {
                    let remaining_leaves = u32::try_from(last.saturating_sub(index)).unwrap_or(0);
                    let rounded = (weight * 10_000.0).round().clamp(1.0, 10_000.0) as u32;
                    let basis = rounded.min(remaining.saturating_sub(remaining_leaves));
                    remaining = remaining.saturating_sub(basis);
                    basis
                };
                (id, basis)
            })
            .collect()
    }
}

/// Why a split/remove was rejected (the host maps these to its own UI affordances).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    /// Unknown cell id.
    NotFound,
    /// The last remaining cell cannot be removed.
    LastCell,
    /// Malformed layout, non-finite resize, or exhausted browser-compatible cell identities.
    InvalidLayout,
}

/// The split-grid model: an authoritative binary tree with stable cell identities.
pub struct Workspace {
    root: WorkspaceLayout,
    next_id: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    /// A workspace holding a single chart cell.
    pub fn new() -> Self {
        Self {
            root: WorkspaceLayout::Cell { id: 1 },
            next_id: 2,
        }
    }

    /// A workspace rooted at a host-issued stable identity.
    pub fn new_with_id(id: u64) -> Result<Self, WorkspaceError> {
        if id == 0 {
            return Err(WorkspaceError::InvalidLayout);
        }
        Ok(Self {
            root: WorkspaceLayout::Cell { id },
            next_id: id.checked_add(1).ok_or(WorkspaceError::InvalidLayout)?,
        })
    }

    /// Restore a typed stable-identity layout without replaying mutations in the host.
    pub fn from_layout(root: &WorkspaceLayout) -> Result<Self, WorkspaceError> {
        let mut ids = std::collections::HashSet::new();
        let max_id = validate_layout(root, 0, &mut ids)?;
        let next_id = max_id.checked_add(1).ok_or(WorkspaceError::InvalidLayout)?;
        Ok(Self {
            root: root.clone(),
            next_id,
        })
    }

    /// Restore only the generic split topology and stable cell identities. Chart state remains
    /// independently owned by each host-created chart and is composed by the browser grid.
    pub fn from_layout_json(json: &str) -> Result<Self, WorkspaceError> {
        let root = serde_json::from_str::<WorkspaceLayout>(json)
            .map_err(|_| WorkspaceError::InvalidLayout)?;
        Self::from_layout(&root)
    }

    /// Atomically replace this workspace's topology with a validated persisted layout.
    pub fn restore_layout_json(&mut self, json: &str) -> Result<(), WorkspaceError> {
        *self = Self::from_layout_json(json)?;
        Ok(())
    }

    /// Leaf cell ids in layout order (left-to-right, top-to-bottom).
    pub fn cell_ids(&self) -> Vec<u64> {
        let mut out = Vec::new();
        self.root.cell_ids(&mut out);
        out
    }

    pub fn chart_count(&self) -> usize {
        self.cell_ids().len()
    }

    /// The identity the next [`Self::split`] will issue, so a host with a narrower id space can
    /// refuse the split before mutating the layout.
    pub fn next_cell_id(&self) -> u64 {
        self.next_id
    }

    /// Split a cell in two; the existing chart keeps its state in the first half and the new
    /// cell (returned id) fills the second. `NotFound` for an unknown cell.
    pub fn split(&mut self, id: u64, direction: SplitDirection) -> Result<u64, WorkspaceError> {
        let new_id = self.next_id;
        self.split_with_id(id, direction, new_id)?;
        Ok(new_id)
    }

    /// Split a leaf using a host-issued stable identity.
    pub fn split_with_id(
        &mut self,
        id: u64,
        direction: SplitDirection,
        new_id: u64,
    ) -> Result<(), WorkspaceError> {
        let next_id = new_id.checked_add(1).ok_or(WorkspaceError::InvalidLayout)?;
        if new_id == 0 || self.cell_ids().contains(&new_id) {
            return Err(WorkspaceError::InvalidLayout);
        }
        if !split_node(&mut self.root, id, direction, new_id) {
            return Err(WorkspaceError::NotFound);
        }
        self.next_id = self.next_id.max(next_id);
        Ok(())
    }

    /// Remove a cell: its sibling subtree absorbs the freed space. `LastCell` refuses to
    /// remove the final chart.
    pub fn remove(&mut self, id: u64) -> Result<(), WorkspaceError> {
        if !self.cell_ids().contains(&id) {
            return Err(WorkspaceError::NotFound);
        }
        if self.chart_count() <= 1 {
            return Err(WorkspaceError::LastCell);
        }
        if !remove_node(&mut self.root, id) {
            return Err(WorkspaceError::NotFound);
        }
        Ok(())
    }

    /// Drag the divider between two adjacent cells: adjusts the ratio of the split node whose
    /// `a` subtree ends at `left_id` and whose `b` subtree starts at `right_id` by
    /// `delta_ratio` (clamped so neither side collapses under ~5%). `NotFound` when no such
    /// divider exists.
    pub fn resize_between(
        &mut self,
        left_id: u64,
        right_id: u64,
        delta_ratio: f64,
    ) -> Result<(), WorkspaceError> {
        if !delta_ratio.is_finite() {
            return Err(WorkspaceError::InvalidLayout);
        }
        if resize_between_node(&mut self.root, left_id, right_id, delta_ratio) {
            Ok(())
        } else {
            Err(WorkspaceError::NotFound)
        }
    }

    /// Set one adjacent boundary to an absolute normalized ratio.
    pub fn resize_between_to(
        &mut self,
        left_id: u64,
        right_id: u64,
        ratio: f64,
    ) -> Result<(), WorkspaceError> {
        if !ratio.is_finite() {
            return Err(WorkspaceError::InvalidLayout);
        }
        let current = self
            .root
            .boundary_ratio(left_id, right_id)
            .ok_or(WorkspaceError::NotFound)?;
        self.resize_between(left_id, right_id, ratio.clamp(0.05, 0.95) - current)
    }

    /// A cloned typed snapshot of the current workspace layout.
    pub fn layout(&self) -> WorkspaceLayout {
        self.root.clone()
    }

    /// The layout tree as JSON (cells by id, splits with direction) for host DOM placement.
    /// This preserves the serialized shape of [`Workspace::layout`].
    pub fn layout_json(&self) -> String {
        serde_json::to_string(&self.root).unwrap_or_else(|_| "{}".to_string())
    }
}

const MAX_RESTORED_CELLS: usize = 1024;
const MAX_RESTORED_DEPTH: usize = 64;

fn validate_layout(
    node: &WorkspaceLayout,
    depth: usize,
    ids: &mut std::collections::HashSet<u64>,
) -> Result<u64, WorkspaceError> {
    if depth > MAX_RESTORED_DEPTH || ids.len() >= MAX_RESTORED_CELLS {
        return Err(WorkspaceError::InvalidLayout);
    }
    match node {
        WorkspaceLayout::Cell { id } => {
            if *id == 0 || !ids.insert(*id) {
                return Err(WorkspaceError::InvalidLayout);
            }
            Ok(*id)
        }
        WorkspaceLayout::Split { ratio, a, b, .. } => {
            if !ratio.is_finite() || !(0.05..=0.95).contains(ratio) {
                return Err(WorkspaceError::InvalidLayout);
            }
            let a_max = validate_layout(a, depth + 1, ids)?;
            let b_max = validate_layout(b, depth + 1, ids)?;
            Ok(a_max.max(b_max))
        }
    }
}

fn collect_weights(node: &WorkspaceLayout, weight: f64, out: &mut Vec<(u64, f64)>) {
    match node {
        WorkspaceLayout::Cell { id } => out.push((*id, weight)),
        WorkspaceLayout::Split { ratio, a, b, .. } => {
            collect_weights(a, weight * *ratio, out);
            collect_weights(b, weight * (1.0 - *ratio), out);
        }
    }
}

/// Replace leaf `id` with a split of `[id, new_id]`; returns whether the leaf existed.
fn split_node(node: &mut WorkspaceLayout, id: u64, direction: SplitDirection, new_id: u64) -> bool {
    match node {
        WorkspaceLayout::Cell { id: cell } if *cell == id => {
            *node = WorkspaceLayout::Split {
                direction,
                ratio: 0.5,
                a: Box::new(WorkspaceLayout::Cell { id }),
                b: Box::new(WorkspaceLayout::Cell { id: new_id }),
            };
            true
        }
        WorkspaceLayout::Cell { .. } => false,
        WorkspaceLayout::Split { a, b, .. } => {
            split_node(a, id, direction, new_id) || split_node(b, id, direction, new_id)
        }
    }
}

/// Adjust the ratio of the split node whose `a` subtree's last leaf is `left_id` and whose
/// `b` subtree's first leaf is `right_id`; clamped to [0.05, 0.95] so neither side collapses.
fn resize_between_node(
    node: &mut WorkspaceLayout,
    left_id: u64,
    right_id: u64,
    delta_ratio: f64,
) -> bool {
    let WorkspaceLayout::Split { a, b, ratio, .. } = node else {
        return false;
    };
    if a.last_leaf() == left_id && b.first_leaf() == right_id {
        *ratio = (*ratio + delta_ratio).clamp(0.05, 0.95);
        return true;
    }
    resize_between_node(a, left_id, right_id, delta_ratio)
        || resize_between_node(b, left_id, right_id, delta_ratio)
}

/// Remove leaf `id`, collapsing its parent split in place (the sibling subtree absorbs the
/// freed slot — at the root this replaces the whole tree with the sibling). Returns whether
/// the leaf existed.
fn remove_node(node: &mut WorkspaceLayout, id: u64) -> bool {
    let WorkspaceLayout::Split { a, b, .. } = node else {
        return false;
    };
    if matches!(**a, WorkspaceLayout::Cell { id: cell } if cell == id) {
        // The split node collapses: the b subtree absorbs the freed slot.
        let absorbed = std::mem::replace(b, Box::new(WorkspaceLayout::Cell { id: u64::MAX }));
        *node = *absorbed;
        return true;
    }
    if matches!(**b, WorkspaceLayout::Cell { id: cell } if cell == id) {
        let absorbed = std::mem::replace(a, Box::new(WorkspaceLayout::Cell { id: u64::MAX }));
        *node = *absorbed;
        return true;
    }
    remove_node(a, id) || remove_node(b, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_finite_resize_preserves_a_restorable_layout() {
        let mut ws = Workspace::new();
        ws.split(1, SplitDirection::Horizontal).unwrap();
        let before = ws.layout_json();
        for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                ws.resize_between(1, 2, delta),
                Err(WorkspaceError::InvalidLayout)
            );
            assert_eq!(ws.layout_json(), before);
            assert!(Workspace::from_layout_json(&ws.layout_json()).is_ok());
        }
    }

    #[test]
    fn exhausted_cell_ids_reject_split_atomically() {
        let mut ws = Workspace::from_layout(&WorkspaceLayout::Cell { id: u64::MAX - 1 }).unwrap();
        assert_eq!(
            ws.split(u64::MAX - 1, SplitDirection::Horizontal),
            Err(WorkspaceError::InvalidLayout)
        );
        let before = ws.layout_json();
        assert_eq!(
            ws.split_with_id(u64::MAX - 1, SplitDirection::Vertical, u64::MAX),
            Err(WorkspaceError::InvalidLayout)
        );
        assert_eq!(ws.layout_json(), before);
        assert!(Workspace::from_layout_json(&ws.layout_json()).is_ok());
    }

    #[test]
    fn split_grows_the_tree_in_layout_order() {
        let mut ws = Workspace::new();
        assert_eq!(ws.cell_ids(), [1]);
        let second = ws.split(1, SplitDirection::Horizontal).unwrap();
        assert_eq!(second, 2);
        assert_eq!(ws.cell_ids(), [1, 2]);
        let third = ws.split(2, SplitDirection::Vertical).unwrap();
        assert_eq!(ws.cell_ids(), [1, 2, 3]);
        let layout = ws.layout_json();
        assert!(layout.contains(r#""direction":"horizontal""#));
        assert!(layout.contains(r#""direction":"vertical""#));
        assert_eq!(ws.chart_count(), 3);
        assert_eq!(third, 3);
    }

    #[test]
    fn persisted_layout_restores_stable_ids_ratios_and_next_identity() {
        let json = r#"{"kind":"split","direction":"horizontal","ratio":0.3,"a":{"kind":"cell","id":4},"b":{"kind":"cell","id":9}}"#;
        let mut ws = Workspace::from_layout_json(json).unwrap();
        assert_eq!(ws.cell_ids(), [4, 9]);
        assert_eq!(ws.layout_json(), json);
        assert_eq!(ws.split(4, SplitDirection::Vertical).unwrap(), 10);
    }

    #[test]
    fn typed_layout_accepts_host_issued_ids_and_reports_weights() {
        let layout = WorkspaceLayout::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.25,
            a: Box::new(WorkspaceLayout::Cell { id: 9_000_000_000 }),
            b: Box::new(WorkspaceLayout::Cell { id: 17 }),
        };
        let mut ws = Workspace::from_layout(&layout).unwrap();
        assert_eq!(ws.layout().leaf_ids(), [9_000_000_000, 17]);
        assert_eq!(
            ws.layout().basis_points(),
            [(9_000_000_000, 2_500), (17, 7_500)]
        );

        ws.split_with_id(17, SplitDirection::Vertical, 42).unwrap();
        assert_eq!(ws.cell_ids(), [9_000_000_000, 17, 42]);
        ws.resize_between_to(17, 42, 0.8).unwrap();
        assert_eq!(ws.layout().boundary_ratio(17, 42), Some(0.8));
    }

    #[test]
    fn persisted_layout_rejects_duplicate_ids_and_invalid_ratios() {
        let duplicate = r#"{"kind":"split","direction":"horizontal","ratio":0.5,"a":{"kind":"cell","id":1},"b":{"kind":"cell","id":1}}"#;
        let bad_ratio = r#"{"kind":"split","direction":"horizontal","ratio":1.0,"a":{"kind":"cell","id":1},"b":{"kind":"cell","id":2}}"#;
        assert_eq!(
            Workspace::from_layout_json(duplicate).err(),
            Some(WorkspaceError::InvalidLayout)
        );
        assert_eq!(
            Workspace::from_layout_json(bad_ratio).err(),
            Some(WorkspaceError::InvalidLayout)
        );
    }

    #[test]
    fn typed_layout_snapshot_tracks_splits_resizes_and_removals() {
        let mut ws = Workspace::new();
        assert_eq!(ws.layout(), WorkspaceLayout::Cell { id: 1 });

        let second = ws.split(1, SplitDirection::Horizontal).unwrap();
        assert_eq!(
            ws.layout(),
            WorkspaceLayout::Split {
                direction: SplitDirection::Horizontal,
                ratio: 0.5,
                a: Box::new(WorkspaceLayout::Cell { id: 1 }),
                b: Box::new(WorkspaceLayout::Cell { id: second }),
            }
        );

        ws.resize_between(1, second, 0.25).unwrap();
        let third = ws.split(second, SplitDirection::Vertical).unwrap();
        assert_eq!(
            ws.layout(),
            WorkspaceLayout::Split {
                direction: SplitDirection::Horizontal,
                ratio: 0.75,
                a: Box::new(WorkspaceLayout::Cell { id: 1 }),
                b: Box::new(WorkspaceLayout::Split {
                    direction: SplitDirection::Vertical,
                    ratio: 0.5,
                    a: Box::new(WorkspaceLayout::Cell { id: second }),
                    b: Box::new(WorkspaceLayout::Cell { id: third }),
                }),
            }
        );

        ws.remove(second).unwrap();
        assert_eq!(
            ws.layout(),
            WorkspaceLayout::Split {
                direction: SplitDirection::Horizontal,
                ratio: 0.75,
                a: Box::new(WorkspaceLayout::Cell { id: 1 }),
                b: Box::new(WorkspaceLayout::Cell { id: third }),
            }
        );
    }

    #[test]
    fn remove_collapses_the_split_node_and_the_last_cell_refuses() {
        let mut ws = Workspace::new();
        let second = ws.split(1, SplitDirection::Horizontal).unwrap();
        let third = ws.split(second, SplitDirection::Vertical).unwrap();
        // Remove the middle: cell 1 and cell 3 stay, the tree is a horizontal split again.
        ws.remove(second).unwrap();
        assert_eq!(ws.cell_ids(), [1, 3]);
        assert!(!ws.layout_json().contains("vertical"));
        // Root collapse: removing cell 1 leaves cell 3 as the only (root) cell.
        ws.remove(1).unwrap();
        assert_eq!(ws.cell_ids(), [3]);
        assert_eq!(ws.remove(third), Err(WorkspaceError::LastCell));
        assert_eq!(ws.remove(42), Err(WorkspaceError::NotFound));
    }

    #[test]
    fn remove_placeholder_is_not_a_real_cell() {
        let mut ws = Workspace::new();
        ws.split(1, SplitDirection::Horizontal).unwrap();
        ws.split(2, SplitDirection::Horizontal).unwrap();
        ws.remove(2).unwrap();
        // The collapse must not leave the placeholder id anywhere.
        assert!(!ws.layout_json().contains(&u64::MAX.to_string()));
        assert_eq!(ws.cell_ids(), [1, 3]);
    }

    #[test]
    fn resize_between_adjusts_the_shared_divider_and_clamps() {
        let mut ws = Workspace::new();
        ws.split(1, SplitDirection::Horizontal).unwrap();
        let third = ws.split(2, SplitDirection::Vertical).unwrap();
        // Nested divider between cells 2 and 3 (the vertical split's own boundary)…
        ws.resize_between(2, third, 0.3).unwrap();
        assert!(ws.layout_json().contains(r#""ratio":0.8"#));
        // …and the outer divider between cell 1 and the (2,3) subtree (its first leaf is 2).
        ws.resize_between(1, 2, -0.2).unwrap();
        assert!(ws.layout_json().contains(r#""ratio":0.3"#));
        // Clamped: a huge drag cannot collapse a side.
        ws.resize_between(1, 2, -1.0).unwrap();
        assert!(ws.layout_json().contains(r#""ratio":0.05"#));
        ws.resize_between(1, 2, 1.0).unwrap();
        assert!(ws.layout_json().contains(r#""ratio":0.95"#));
        // No divider between non-adjacent cells.
        assert_eq!(
            ws.resize_between(1, third, 0.1),
            Err(WorkspaceError::NotFound)
        );
        // Ratios survive an unrelated split.
        ws.split(1, SplitDirection::Horizontal).unwrap();
        assert!(ws.layout_json().contains(r#""ratio":0.95"#));
    }
}
