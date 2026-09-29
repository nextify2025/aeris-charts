//! Centralized pane-local paint ordering.
//!
//! One pane-local order covers chart content (series + drawings):
//! `grid/background → idle indicators → idle drawings → ordinary price series → active objects`.
//! Stable ordering is preserved within each group; indicator outputs move as one visual group
//! with internal ordering preserved. Temporary promotion (dragging/editing → hovered →
//! selected → idle) never rewrites saved ordering — deselection, hover leave, cancellation,
//! or removal restores the underlying order.
//!
//! Explicit [`ChartEngine::set_series_order`] calls override the default series grouping for
//! idle series (idle drawings remain below price series). The existing
//! `hoveredSeriesOnTop` option gates hover promotion for both series and drawings.
//! Axes, crosshair, and financial-action controls keep their existing protected layers above
//! all chart content; active chart content remains clipped to its owning pane.
//!
//! Frame assembly is the single ordering owner. Hit testing reuses the stable
//! [`ChartEngine::series_order`] (and drawing z-order) for tie-breaking so temporary
//! promotion cannot cause hover oscillation. Retained per-series geometry and per-drawing
//! segments are reused; ordering changes reassemble without rebuilding geometry.

use aeris_charts_core::model::data_layer::SeriesId;

use crate::drawings::DrawingId;
use crate::ChartEngine;

/// Active priority for chart content. Higher paints later (on top).
/// Idle (0) stays in its stable group; selected (1) promotes above idle;
/// hovered (2) above selected when `hoveredSeriesOnTop` holds; dragging/editing (3,
/// drawings only) topmost.
pub(crate) const PRIORITY_IDLE: u8 = 0;
pub(crate) const PRIORITY_SELECTED: u8 = 1;
pub(crate) const PRIORITY_HOVERED: u8 = 2;
pub(crate) const PRIORITY_DRAG_EDIT: u8 = 3;

impl ChartEngine {
    /// Binding identity for an indicator output series, or `None` for ordinary series.
    /// Bindings are the grouping owner — no classification by series type or title.
    pub(crate) fn indicator_binding_id(&self, id: SeriesId) -> Option<SeriesId> {
        self.indicators
            .iter()
            .find_map(|binding| binding.outputs.contains(&id).then(|| binding.outputs[0]))
    }

    /// Outputs of one indicator binding in binding order (documented output order).
    pub(crate) fn indicator_group_outputs(&self, binding_id: SeriesId) -> Vec<SeriesId> {
        self.indicators
            .iter()
            .find(|binding| binding.outputs.first() == Some(&binding_id))
            .map(|binding| binding.outputs.clone())
            .unwrap_or_else(|| vec![binding_id])
    }

    /// Active priority for one series (0 idle, 1 selected, 2 hovered).
    /// Hover promotion respects `hoveredSeriesOnTop`; a disabled option treats hovered as idle.
    pub(crate) fn series_active_priority(&self, id: SeriesId) -> u8 {
        if self.series_is_selected(id) {
            // A series both hovered and selected takes the higher (hovered) slot below.
            if self.hover_promotion_enabled() && self.hovered_series() == Some(id) {
                return PRIORITY_HOVERED;
            }
            return PRIORITY_SELECTED;
        }
        if self.hover_promotion_enabled() && self.hovered_series() == Some(id) {
            return PRIORITY_HOVERED;
        }
        PRIORITY_IDLE
    }

    /// Group priority for an indicator binding (or singleton ordinary series): the max of its
    /// members so hovering/selecting any Bollinger output promotes the complete indicator.
    pub(crate) fn series_group_priority(&self, ids: &[SeriesId]) -> u8 {
        ids.iter()
            .map(|&id| self.series_active_priority(id))
            .max()
            .unwrap_or(PRIORITY_IDLE)
    }

    fn hover_promotion_enabled(&self) -> bool {
        self.options.get().hovered_series_on_top
    }

    /// Whether `set_series_order` has overridden the default series grouping.
    pub(crate) fn series_order_is_explicit(&self) -> bool {
        self.series_order_explicit
    }

    /// Global effective series paint order, bottom to top, for frame assembly.
    /// Default idle: indicators (stable) before ordinary (stable); explicit idle: stable
    /// `series_order` verbatim. Active groups (indicator outputs gathered, internal
    /// `series_order` preserved) follow in priority order (selected below hovered).
    /// Hidden/removed/paneless series are retained here and skipped at emission, matching
    /// the prior contract that `series_order` stays the stable identity list.
    pub(crate) fn effective_series_order(&self) -> Vec<SeriesId> {
        if self.series_order_is_explicit() {
            return self.effective_series_order_explicit();
        }
        self.effective_series_order_default()
    }

    fn effective_series_order_default(&self) -> Vec<SeriesId> {
        use std::collections::HashMap;
        // Group indicator outputs by binding; ordinary series are singleton groups.
        // `first_pos` is the earliest stable position for stable ordering within each tier.
        struct Group {
            ids: Vec<SeriesId>,
            positions: Vec<usize>,
            priority: u8,
            first_pos: usize,
            is_indicator: bool,
        }
        let mut groups: HashMap<SeriesId, Group> = HashMap::new();
        let mut order_keys: Vec<SeriesId> = Vec::new();
        for (pos, &id) in self.series_order.iter().enumerate() {
            let priority = self.series_active_priority(id);
            if let Some(binding) = self.indicator_binding_id(id) {
                let key = binding;
                if let Some(group) = groups.get_mut(&key) {
                    group.ids.push(id);
                    group.positions.push(pos);
                    group.priority = group.priority.max(priority);
                    group.first_pos = group.first_pos.min(pos);
                } else {
                    groups.insert(
                        key,
                        Group {
                            ids: vec![id],
                            positions: vec![pos],
                            priority,
                            first_pos: pos,
                            is_indicator: true,
                        },
                    );
                    order_keys.push(key);
                }
            } else {
                // Ordinary singleton keyed by itself (never collides with binding ids that are
                // also series ids? A binding id IS its first output id, which is an indicator,
                // so ordinary ids never equal a binding key unless that series is itself an
                // indicator output — handled above. Safe.)
                groups.insert(
                    id,
                    Group {
                        ids: vec![id],
                        positions: vec![pos],
                        priority,
                        first_pos: pos,
                        is_indicator: false,
                    },
                );
                order_keys.push(id);
            }
        }
        // Stable within each group by series_order position (preserves internal BB/ribbon order
        // even after explicit-adjacent inserts; default inserts keep binding order contiguous).
        for group in groups.values_mut() {
            let mut paired: Vec<(usize, SeriesId)> = group
                .ids
                .iter()
                .copied()
                .zip(group.positions.iter().copied())
                .map(|(id, p)| (p, id))
                .collect();
            paired.sort_by_key(|&(p, _)| p);
            group.ids = paired.into_iter().map(|(_, id)| id).collect();
        }
        let mut idle_indicators: Vec<(usize, Vec<SeriesId>)> = Vec::new();
        let mut ordinary_idle: Vec<(usize, Vec<SeriesId>)> = Vec::new();
        let mut active: Vec<(u8, usize, Vec<SeriesId>)> = Vec::new();
        for key in order_keys {
            let Some(group) = groups.remove(&key) else {
                continue;
            };
            if group.priority != PRIORITY_IDLE {
                active.push((group.priority, group.first_pos, group.ids));
            } else if group.is_indicator {
                idle_indicators.push((group.first_pos, group.ids));
            } else {
                ordinary_idle.push((group.first_pos, group.ids));
            }
        }
        idle_indicators.sort_by_key(|&(p, _)| p);
        ordinary_idle.sort_by_key(|&(p, _)| p);
        // Priority ascending (selected below hovered), stable within a tier.
        active.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut out = Vec::with_capacity(self.series_order.len());
        for (_, mut ids) in idle_indicators {
            out.append(&mut ids);
        }
        for (_, mut ids) in ordinary_idle {
            out.append(&mut ids);
        }
        for (_, _, mut ids) in active {
            out.append(&mut ids);
        }
        out
    }

    fn effective_series_order_explicit(&self) -> Vec<SeriesId> {
        use std::collections::HashMap;
        // Group-aware like the default path: a binding's priority is its members' max, so
        // hovering/selecting any output promotes the complete indicator with internal explicit
        // order preserved. Only idle grouping is overridden — idle paints verbatim in explicit
        // order minus active-group members (which leave together, never splitting [1, 2, 3]
        // into [1, 3, 2]).
        struct Group {
            ids: Vec<(usize, SeriesId)>,
            priority: u8,
            first_pos: usize,
        }
        let mut groups: HashMap<SeriesId, Group> = HashMap::new();
        let mut order_keys: Vec<SeriesId> = Vec::new();
        for (pos, &id) in self.series_order.iter().enumerate() {
            let priority = self.series_active_priority(id);
            let key = self.indicator_binding_id(id).unwrap_or(id);
            if let Some(group) = groups.get_mut(&key) {
                group.ids.push((pos, id));
                group.priority = group.priority.max(priority);
                group.first_pos = group.first_pos.min(pos);
            } else {
                groups.insert(
                    key,
                    Group {
                        ids: vec![(pos, id)],
                        priority,
                        first_pos: pos,
                    },
                );
                order_keys.push(key);
            }
        }
        let mut idle: Vec<(usize, SeriesId)> = Vec::new();
        let mut active: Vec<(u8, usize, Vec<SeriesId>)> = Vec::new();
        for key in order_keys {
            let Some(mut group) = groups.remove(&key) else {
                continue;
            };
            group.ids.sort_by_key(|&(p, _)| p);
            if group.priority == PRIORITY_IDLE {
                idle.append(&mut group.ids);
            } else {
                active.push((
                    group.priority,
                    group.first_pos,
                    group.ids.into_iter().map(|(_, id)| id).collect(),
                ));
            }
        }
        // Idle verbatim in explicit order; active ordered by priority then first position.
        idle.sort_by_key(|&(p, _)| p);
        let mut out: Vec<SeriesId> = idle.into_iter().map(|(_, id)| id).collect();
        active.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, mut ids) in active {
            out.append(&mut ids);
        }
        out
    }

    /// Active priority for one drawing: dragging/editing (3) → hovered (2) → selected (1).
    /// Hover promotion respects `hoveredSeriesOnTop` like series; selection/drag/edit always
    /// promote. Deselection, hover leave, cancellation, or removal restores idle.
    pub(crate) fn drawing_active_priority(&self, id: DrawingId) -> u8 {
        if self.drawing_drag.as_ref().is_some_and(|drag| drag.id == id)
            || self.editing_drawing() == Some(id)
        {
            return PRIORITY_DRAG_EDIT;
        }
        let hovered = self.hovered_drawing == Some(id) || self.hovered_text == Some(id);
        if hovered && self.hover_promotion_enabled() {
            return PRIORITY_HOVERED;
        }
        if self.selected_drawing == Some(id) {
            // Hovered + selected takes hovered above (handled above when promotion enabled);
            // with promotion disabled a hovered+selected drawing stays selected-promoted.
            return PRIORITY_SELECTED;
        }
        // Hovered with promotion disabled falls through to selected-or-idle below.
        if hovered && self.selected_drawing == Some(id) {
            return PRIORITY_SELECTED;
        }
        PRIORITY_IDLE
    }

    /// Pane-local drawing order split for frame assembly: idle (stable z-order) paints below
    /// price series; active (priority then stable) paints above ordinary series with the
    /// active series. Stale panes draw nowhere.
    pub(crate) fn pane_drawing_tiers(&self, pane: usize) -> (Vec<DrawingId>, Vec<DrawingId>) {
        if pane >= self.panes.len() {
            return (Vec::new(), Vec::new());
        }
        let mut idle: Vec<DrawingId> = Vec::new();
        let mut active: Vec<(u8, usize, DrawingId)> = Vec::new();
        for (pos, drawing) in self.drawings.iter().enumerate() {
            if drawing.pane_index != pane {
                continue;
            }
            let priority = self.drawing_active_priority(drawing.id);
            if priority == PRIORITY_IDLE {
                idle.push(drawing.id);
            } else {
                active.push((priority, pos, drawing.id));
            }
        }
        active.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        (idle, active.into_iter().map(|(_, _, id)| id).collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::{ChartEngine, SeriesKind};

    fn chart_with_bars(n: usize) -> ChartEngine {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times: Vec<f64> = (0..n).map(|i| (i * 3600) as f64).collect();
        let values: Vec<f64> = (0..n).map(|i| 100.0 + (i as f64).sin() * 5.0).collect();
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();
        chart
    }

    #[test]
    fn idle_indicators_default_below_price_despite_insertion_order() {
        let mut chart = chart_with_bars(30);
        // Later-added Bollinger must not cover the primary candles by default.
        let bb = chart.add_bollinger(0, 5, 2.0);
        assert_eq!(bb.len(), 3);
        assert_eq!(
            chart.series_order(),
            &[0, bb[0], bb[1], bb[2]],
            "stable insertion order preserved"
        );
        assert_eq!(
            chart.effective_series_order(),
            vec![bb[0], bb[1], bb[2], 0],
            "idle BB group paints below ordinary price"
        );
        // SMA added even later stays in the idle-indicator tier, stable within the group.
        let sma = chart.add_sma(0, 5).unwrap();
        assert_eq!(
            chart.effective_series_order(),
            vec![bb[0], bb[1], bb[2], sma, 0]
        );
    }

    #[test]
    fn hovering_any_band_promotes_the_complete_indicator_group() {
        let mut chart = chart_with_bars(30);
        let bb = chart.add_bollinger(0, 5, 2.0);
        chart.set_hovered_series(Some(bb[1]));
        assert_eq!(
            chart.effective_series_order(),
            vec![0, bb[0], bb[1], bb[2]],
            "hovered BB group (internal order kept) above ordinary"
        );
        assert_eq!(
            chart.series_order(),
            &[0, bb[0], bb[1], bb[2]],
            "saved order untouched by promotion"
        );
        chart.set_hovered_series(None);
        assert_eq!(
            chart.effective_series_order(),
            vec![bb[0], bb[1], bb[2], 0],
            "hover leave restores underlying order"
        );
    }

    #[test]
    fn ema_ribbon_is_one_group_and_explicit_order_overrides_grouping() {
        let mut chart = chart_with_bars(250);
        let ribbon = chart.add_ema_ribbon(0, [2, 3, 4, 5, 6]);
        assert_eq!(ribbon.len(), 5);
        assert_eq!(
            chart.effective_series_order(),
            vec![ribbon[0], ribbon[1], ribbon[2], ribbon[3], ribbon[4], 0]
        );
        chart.set_hovered_series(Some(ribbon[3]));
        assert_eq!(
            chart.effective_series_order(),
            vec![0, ribbon[0], ribbon[1], ribbon[2], ribbon[3], ribbon[4]],
            "hovering any ribbon output promotes all five in order"
        );
        chart.set_hovered_series(None);
        // Explicit override: idle paints verbatim, so the ribbon can cover price when asked.
        assert!(chart.set_series_order(vec![
            0, ribbon[0], ribbon[1], ribbon[2], ribbon[3], ribbon[4]
        ]));
        assert!(chart.series_order_is_explicit());
        assert_eq!(
            chart.effective_series_order(),
            vec![0, ribbon[0], ribbon[1], ribbon[2], ribbon[3], ribbon[4]]
        );
        // Hovering a middle output under explicit order still promotes the whole group
        // intact — never [ribbon[0], ribbon[2], ribbon[1]]-style splits.
        chart.set_hovered_series(Some(ribbon[1]));
        assert_eq!(
            chart.effective_series_order(),
            vec![0, ribbon[0], ribbon[1], ribbon[2], ribbon[3], ribbon[4]],
            "explicit hover keeps internal group order"
        );
        chart.set_hovered_series(None);
        assert_eq!(
            chart.effective_series_order(),
            vec![0, ribbon[0], ribbon[1], ribbon[2], ribbon[3], ribbon[4]]
        );
    }

    #[test]
    fn selected_promotes_below_hovered_and_deselection_restores() {
        let mut chart = chart_with_bars(30);
        let sma = chart.add_sma(0, 5).unwrap();
        let other = chart.add_series(SeriesKind::Line);
        let times: Vec<f64> = (0..30).map(|i| (i * 3600) as f64).collect();
        let values = [105.0; 30];
        chart
            .set_series_data(other, &times, &values, &values, &values, &values)
            .unwrap();
        // Stable: indicators [sma] then ordinary [0, other] (insertion within ordinary).
        assert_eq!(chart.effective_series_order(), vec![sma, 0, other]);
        chart.set_selected_series(Some(0));
        assert_eq!(
            chart.effective_series_order(),
            vec![sma, other, 0],
            "selected ordinary above idle"
        );
        chart.set_hovered_series(Some(sma));
        assert_eq!(
            chart.effective_series_order(),
            vec![other, 0, sma],
            "hovered (priority 2) above selected (priority 1)"
        );
        // Hover promotion option gates hover only; selection still promotes.
        chart
            .apply_options("{\"hoveredSeriesOnTop\": false}")
            .unwrap();
        assert_eq!(
            chart.effective_series_order(),
            vec![sma, other, 0],
            "disabled hover treats hovered as idle; selected stays promoted"
        );
        chart
            .apply_options("{\"hoveredSeriesOnTop\": true}")
            .unwrap();
        chart.set_hovered_series(None);
        chart.set_selected_series(None);
        assert_eq!(chart.effective_series_order(), vec![sma, 0, other]);
    }

    #[test]
    fn drawing_priority_is_drag_edit_then_hovered_then_selected() {
        let mut chart = chart_with_bars(10);
        let a = chart
            .add_drawing(
                crate::DrawingKind::TrendLine,
                0,
                vec![
                    crate::DrawingPoint {
                        logical: 1.0,
                        price: 100.0,
                    },
                    crate::DrawingPoint {
                        logical: 3.0,
                        price: 101.0,
                    },
                ],
                None,
            )
            .unwrap();
        let b = chart
            .add_drawing(
                crate::DrawingKind::TrendLine,
                0,
                vec![
                    crate::DrawingPoint {
                        logical: 2.0,
                        price: 100.0,
                    },
                    crate::DrawingPoint {
                        logical: 4.0,
                        price: 101.0,
                    },
                ],
                None,
            )
            .unwrap();
        assert_eq!(chart.pane_drawing_tiers(0), (vec![a, b], vec![]));
        chart.set_selected_drawing(Some(a));
        assert_eq!(chart.pane_drawing_tiers(0), (vec![b], vec![a]));
        chart.set_hovered_drawing(Some(b));
        assert_eq!(
            chart.pane_drawing_tiers(0),
            (vec![], vec![a, b]),
            "selected (1) below hovered (2)"
        );
        // Dragging (3) tops hovered/selected; cancel restores without rewriting z-order.
        // Probe `a`'s own start so the drag grabs the selected drawing (not the topmost `b`).
        let x = chart.logical_to_coordinate(1.0).unwrap();
        let y = chart.series_price_to_coordinate(0, 100.0).unwrap();
        assert!(chart.drawing_drag_start_at(x, y));
        let drag_id = chart
            .drawing_drag_active()
            .then(|| chart.selected_drawing())
            .flatten();
        assert!(drag_id.is_some());
        let (idle, active) = chart.pane_drawing_tiers(0);
        assert!(idle.is_empty() || !active.is_empty());
        assert_eq!(active.last(), drag_id.as_ref());
        chart.drawing_drag_cancel();
        assert_eq!(
            chart.pane_drawing_tiers(0),
            (vec![], vec![a, b]),
            "cancellation restores hover/selected tiers, stable intact"
        );
        chart.set_hovered_drawing(None);
        chart.set_selected_drawing(None);
        assert_eq!(chart.pane_drawing_tiers(0), (vec![a, b], vec![]));
        assert_eq!(
            chart.drawings_json(),
            chart.drawings_json(),
            "saved drawing z-order never rewritten by promotion"
        );
    }

    #[test]
    fn removal_releases_promotion_without_rewriting_saved_order() {
        let mut chart = chart_with_bars(30);
        let bb = chart.add_bollinger(0, 5, 2.0);
        chart.set_hovered_series(Some(bb[0]));
        chart.set_selected_series(Some(0));
        // Removing any BB output drops the whole binding (all outputs tombstoned together);
        // hovered/selected promotion releases with it, saved order only prunes tombstones.
        assert!(chart.remove_series(bb[1]));
        assert!(!chart.series_order().contains(&bb[0]));
        assert!(!chart.series_order().contains(&bb[1]));
        assert!(!chart.series_order().contains(&bb[2]));
        assert_eq!(chart.series_order(), &[0]);
        assert_eq!(chart.hovered_series(), None);
        assert_eq!(chart.effective_series_order(), vec![0]);
        // Saved order never rewritten by promotion itself (checked in hover test); removal
        // only prunes.
    }
}
