//! Multi-chart workspace host: the wasm surface over `aeris_charts_engine::Workspace` (the split-grid
//! MODEL). The browser side keeps DOM placement, commercial limits, usage metering, and one chart
//! instance per cell. The shared engine owns generic topology only.

use aeris_charts_engine::{SplitDirection, Workspace};
use wasm_bindgen::prelude::*;

/// Host-facing workspace handle. Clock is host-injected seconds (Date.now()/1000).
#[wasm_bindgen]
pub struct AerisWorkspace {
    workspace: Workspace,
}

#[wasm_bindgen]
impl AerisWorkspace {
    /// A workspace holding a single chart cell (id 1).
    #[wasm_bindgen(constructor)]
    pub fn new(_now: f64) -> Self {
        Self {
            workspace: Workspace::new(),
        }
    }

    /// Split a cell: `"horizontal"` (side by side) / `"vertical"` (stacked). Returns the new
    /// cell's id, or -1 when the cap rejects it / the cell is unknown.
    pub fn split(&mut self, id: u32, direction: &str, now: f64) -> i64 {
        let direction = match direction {
            "horizontal" => SplitDirection::Horizontal,
            "vertical" => SplitDirection::Vertical,
            _ => return -1,
        };
        let _ = now;
        // Browser cell identities are u32; refuse a split whose new cell could not be addressed.
        if u32::try_from(self.workspace.next_cell_id()).is_err() {
            return -1;
        }
        match self.workspace.split(id as u64, direction) {
            Ok(new_id) => new_id as i64,
            Err(_) => -1,
        }
    }

    /// Remove a cell; false when the id is unknown or it is the last chart.
    pub fn remove(&mut self, id: u32) -> bool {
        matches!(self.workspace.remove(id as u64), Ok(()))
    }

    /// Drag the divider between two adjacent cells: adjusts their shared split's ratio by
    /// `delta_ratio` (the `a` side's share change, clamped to [0.05, 0.95]). False when no
    /// such divider exists.
    pub fn resize_between(&mut self, left_id: u32, right_id: u32, delta_ratio: f64) -> bool {
        self.workspace
            .resize_between(left_id as u64, right_id as u64, delta_ratio)
            .is_ok()
    }

    pub fn chart_count(&self) -> usize {
        self.workspace.chart_count()
    }

    /// Leaf cell ids in layout order as a JSON array.
    pub fn cell_ids_json(&self) -> String {
        serde_json::to_string(&self.workspace.cell_ids()).unwrap_or_else(|_| "[]".to_string())
    }

    /// The layout tree as JSON (cells by id, splits with direction) for host DOM placement.
    pub fn layout_json(&self) -> String {
        self.workspace.layout_json()
    }

    /// Atomically restore a validated split layout with its stable cell ids and ratios.
    pub fn restore_layout_json(&mut self, json: &str) -> bool {
        self.workspace.restore_layout_json(json).is_ok()
    }
}
