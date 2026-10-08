//! Public price-scale state accessors (mode/autoscale/inversion/margins/visible range) for
//! pane, left/right and overlay scales, plus per-series scale binding. Extracted from `lib.rs`.

use super::*;

impl ChartEngine {
    /// Resolve the primary live series for one price scale. Series zero wins when attached;
    /// otherwise the first visible attached series is returned.
    #[must_use]
    pub fn primary_series_on_price_scale(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<PriceScalePrimarySeries> {
        let mut fallback = None;
        for series in &self.series {
            if series.removed || series.pane_index != pane || series.price_scale_target != target {
                continue;
            }
            let snapshot = PriceScalePrimarySeries {
                series_id: series.id,
                price_line_visible: series.price_line_visible,
                last_value_visible: series.last_value_visible,
                title_visible: series.title_visible,
                countdown_visible: series.countdown_visible,
                bid_ask_visible: series.bid_ask_visible,
            };
            if series.id == 0 {
                return Some(snapshot);
            }
            if series.visible && fallback.is_none() {
                fallback = Some(snapshot);
            }
        }
        fallback
    }

    /// Move every live series attached to one built-in price scale to another, carrying the
    /// source scale options and retiring its axis when no series remain.
    pub fn rebind_price_scale_series(
        &mut self,
        pane: usize,
        from: PriceScaleTarget,
        to: PriceScaleTarget,
    ) -> bool {
        if from == to
            || !self
                .panes
                .get(pane)
                .is_some_and(|entry| entry.scale(from).is_some() && entry.scale(to).is_some())
        {
            return false;
        }
        let ids = self
            .series
            .iter()
            .filter(|series| {
                !series.removed && series.pane_index == pane && series.price_scale_target == from
            })
            .map(|series| series.id)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return false;
        }
        let options = self.price_scale_options_json(pane, from);
        let entry = &mut self.panes[pane];
        if let Some(format) = entry.explicit_price_format(from) {
            entry.set_explicit_price_format(to, Some(format));
        }
        for id in ids {
            self.set_series_price_scale(id, to);
        }
        let _ = self.set_price_scale_visible_for(pane, to, true);
        if let Some(options) = options {
            let _ = self.price_scale_apply_options_json(pane, to, &options);
        }
        let from_still_used = self.series.iter().any(|series| {
            !series.removed && series.pane_index == pane && series.price_scale_target == from
        });
        if !from_still_used {
            let _ = self.set_price_scale_visible_for(pane, from, false);
        }
        true
    }

    pub fn price_scale_axis_width(&self, pane: usize, target: PriceScaleTarget) -> Option<f64> {
        self.panes.get(pane)?.scale(target)?;
        if !self.price_scale_visible_for(pane, target) {
            return Some(0.0);
        }
        Some(match target {
            PriceScaleTarget::Right => self.right_builtin_axis_w,
            PriceScaleTarget::Left => self.left_builtin_axis_w,
            PriceScaleTarget::Overlay => 0.0,
            PriceScaleTarget::Named(id) => self.panes[pane].named_scale(id)?.width,
        })
    }

    pub fn price_scale_axis_geometry(
        &self,
        pane_index: usize,
        target: PriceScaleTarget,
    ) -> Option<(PriceScaleSide, f64, f64)> {
        let pane = self.panes.get(pane_index)?;
        let side = pane.scale_side(target)?;
        if !self.price_scale_visible_for(pane_index, target) {
            return None;
        }
        let targets = pane.ordered_side_targets(side);
        let mut offset = 0.0;
        for candidate in targets {
            if !self.price_scale_visible_for(pane_index, candidate) {
                continue;
            }
            let width = self.price_scale_axis_width(pane_index, candidate)?;
            if candidate == target {
                let x = match side {
                    PriceScaleSide::Right => self.pane_left + self.pane_w + offset,
                    PriceScaleSide::Left => self.pane_left - offset - width,
                };
                return Some((side, x, width));
            }
            offset += width;
        }
        None
    }

    pub fn price_axis_target_at(
        &self,
        pane_index: usize,
        plot_relative_x: f64,
    ) -> Option<PriceScaleTarget> {
        let absolute_x = self.pane_left + plot_relative_x;
        let pane = self.panes.get(pane_index)?;
        pane.scale_targets().find(|target| {
            self.price_scale_axis_geometry(pane_index, *target)
                .is_some_and(|(_, x, width)| absolute_x >= x && absolute_x <= x + width)
        })
    }

    pub fn price_scale_target_for_id(&self, pane: usize, id: &str) -> Option<PriceScaleTarget> {
        self.panes.get(pane)?.target_for_public_id(id)
    }

    pub fn price_scale_id_for_target(&self, pane: usize, target: PriceScaleTarget) -> Option<&str> {
        self.panes.get(pane)?.public_id_for_target(target)
    }

    pub fn add_price_scale(
        &mut self,
        pane_index: usize,
        public_id: &str,
        side: PriceScaleSide,
        order: Option<usize>,
        visible: bool,
    ) -> Result<PriceScaleTarget, ChartError> {
        if public_id.is_empty() || public_id == "left" || public_id == "right" {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "named price scale id is reserved or empty",
            ));
        }
        if public_id.len() > MAX_PRICE_SCALE_ID_BYTES {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                format!("price scale id exceeds {MAX_PRICE_SCALE_ID_BYTES} UTF-8 bytes"),
            ));
        }
        let Some(pane) = self.panes.get_mut(pane_index) else {
            return Err(ChartError::new(
                ErrorCode::InvalidHandle,
                "price scale pane is not live",
            ));
        };
        if pane.target_for_public_id(public_id).is_some() {
            return Err(ChartError::new(
                ErrorCode::InvalidOptions,
                "price scale id already exists in this pane",
            ));
        }
        if pane.named_scales.len() >= MAX_NAMED_PRICE_SCALES_PER_PANE {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                format!(
                    "pane already has the maximum of {MAX_NAMED_PRICE_SCALES_PER_PANE} named price scales"
                ),
            ));
        }
        let id = PriceScaleId::try_from(pane.next_price_scale_id)?;
        pane.next_price_scale_id = pane.next_price_scale_id.checked_add(1).ok_or_else(|| {
            ChartError::new(
                ErrorCode::ResourceLimit,
                "price scale identity space is exhausted",
            )
        })?;
        let target = PriceScaleTarget::Named(id);
        pane.named_scales.push(NamedPriceScale {
            id,
            public_id: public_id.to_string(),
            side,
            order: pane.ordered_side_targets(side).len(),
            visible,
            width: 0.0,
            marker_margin_above: 0.0,
            marker_margin_below: 0.0,
            scale: PriceScaleCore::new(PriceScaleCoreOptions::default()),
        });
        pane.move_axis_target(target, side, order.unwrap_or(usize::MAX));
        pane.layout();
        self.invalidate_frame_all();
        Ok(target)
    }

    pub fn move_price_scale(
        &mut self,
        pane_index: usize,
        target: PriceScaleTarget,
        side: PriceScaleSide,
        order: usize,
    ) -> bool {
        let Some(pane) = self.panes.get_mut(pane_index) else {
            return false;
        };
        if !pane.move_axis_target(target, side, order) {
            return false;
        }
        self.invalidate_frame_all();
        true
    }

    pub fn remove_price_scale(
        &mut self,
        pane_index: usize,
        target: PriceScaleTarget,
    ) -> Result<(), ChartError> {
        let PriceScaleTarget::Named(id) = target else {
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "built-in price scales cannot be removed",
            ));
        };
        if self.series.iter().any(|series| {
            !series.removed
                && series.pane_index == pane_index
                && series.price_scale_target == target
        }) {
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "move or remove attached series before removing this price scale",
            ));
        }
        let Some(pane) = self.panes.get_mut(pane_index) else {
            return Err(ChartError::new(
                ErrorCode::InvalidHandle,
                "price scale pane is not live",
            ));
        };
        let Some(index) = pane.named_scales.iter().position(|entry| entry.id == id) else {
            return Err(ChartError::new(
                ErrorCode::StaleHandle,
                "price scale has been removed",
            ));
        };
        let side = pane.named_scales[index].side;
        pane.named_scales.remove(index);
        pane.set_explicit_price_format(target, None);
        let targets = pane.ordered_side_targets(side);
        for (order, candidate) in targets.into_iter().enumerate() {
            pane.set_scale_order(candidate, order);
        }
        self.invalidate_frame_all();
        Ok(())
    }

    pub fn price_scales(&self, pane_index: usize) -> Option<Vec<PriceScaleInfo>> {
        let pane = self.panes.get(pane_index)?;
        let mut targets: Vec<_> = pane.scale_targets().collect();
        targets.sort_by_key(|target| match pane.scale_side(*target) {
            Some(PriceScaleSide::Left) => (0, pane.scale_order(*target).unwrap_or(0)),
            Some(PriceScaleSide::Right) => (1, pane.scale_order(*target).unwrap_or(0)),
            None => (2, 0),
        });
        Some(
            targets
                .into_iter()
                .map(|target| PriceScaleInfo {
                    id: pane.public_id_for_target(target).unwrap_or("").to_string(),
                    side: pane.scale_side(target),
                    order: pane.scale_order(target),
                    visible: self.price_scale_visible_for(pane_index, target),
                    built_in: !matches!(target, PriceScaleTarget::Named(_)),
                    pane_index,
                    series_ids: self
                        .series
                        .iter()
                        .filter(|series| {
                            !series.removed
                                && series.pane_index == pane_index
                                && series.price_scale_target == target
                        })
                        .map(|series| series.id)
                        .collect(),
                })
                .collect(),
        )
    }

    pub fn price_scale_visible_for(&self, pane: usize, target: PriceScaleTarget) -> bool {
        match target {
            PriceScaleTarget::Right => self.options.get().right_price_scale.visible,
            PriceScaleTarget::Left => self.options.get().left_price_scale.visible,
            PriceScaleTarget::Overlay => false,
            PriceScaleTarget::Named(id) => self
                .panes
                .get(pane)
                .and_then(|pane| pane.named_scale(id))
                .is_some_and(|entry| entry.visible),
        }
    }

    pub fn set_price_scale_visible_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        visible: bool,
    ) -> bool {
        match target {
            PriceScaleTarget::Right => self
                .options
                .apply(&serde_json::json!({"rightPriceScale": {"visible": visible}})),
            PriceScaleTarget::Left => self
                .options
                .apply(&serde_json::json!({"leftPriceScale": {"visible": visible}})),
            PriceScaleTarget::Overlay => return !visible,
            PriceScaleTarget::Named(id) => {
                let Some(entry) = self
                    .panes
                    .get_mut(pane)
                    .and_then(|pane| pane.named_scale_mut(id))
                else {
                    return false;
                };
                entry.visible = visible;
            }
        }
        self.invalidate_frame_all();
        true
    }

    pub fn price_scale_for(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<&PriceScaleCore> {
        self.panes.get(pane)?.scale(target)
    }

    pub fn price_scale_for_mut(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<&mut PriceScaleCore> {
        self.panes.get_mut(pane)?.scale_mut(target)
    }

    /// Current visible raw-value range for a pane price scale.
    pub fn price_scale_visible_range(&self, pane: usize, overlay: bool) -> Option<(f64, f64)> {
        self.price_scale_visible_range_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
        )
    }

    pub fn price_scale_visible_range_for(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<(f64, f64)> {
        let range = self.price_scale_for(pane, target)?.price_range_for_api()?;
        Some((range.min_value(), range.max_value()))
    }

    /// Install a manual raw-value range and disable autoscale, matching reference `setVisibleRange`.
    pub fn set_price_scale_visible_range(
        &mut self,
        pane: usize,
        overlay: bool,
        from: f64,
        to: f64,
    ) {
        self.invalidate_frame_scene();
        self.set_price_scale_visible_range_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
            from,
            to,
        );
    }

    pub fn set_price_scale_visible_range_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        from: f64,
        to: f64,
    ) {
        self.invalidate_frame_scene();
        if !from.is_finite() || !to.is_finite() || from >= to {
            return;
        }
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.set_auto_scale(false);
            let range = scale.price_range_from_api(&PriceRange::new(from, to));
            scale.set_price_range(Some(range));
        }
    }

    pub fn price_scale_auto_scale(&self, pane: usize, overlay: bool) -> Option<bool> {
        self.price_scale_auto_scale_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
        )
    }

    pub fn price_scale_auto_scale_for(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<bool> {
        Some(self.price_scale_for(pane, target)?.is_auto_scale())
    }

    pub fn set_price_scale_auto_scale(&mut self, pane: usize, overlay: bool, enabled: bool) {
        self.invalidate_frame_scene();
        self.set_price_scale_auto_scale_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
            enabled,
        );
    }

    pub fn set_price_scale_auto_scale_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        enabled: bool,
    ) {
        self.invalidate_frame_scene();
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.set_auto_scale(enabled);
        }
    }

    pub fn price_scale_inverted(&self, pane: usize, overlay: bool) -> Option<bool> {
        self.price_scale_inverted_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
        )
    }

    pub fn price_scale_inverted_for(&self, pane: usize, target: PriceScaleTarget) -> Option<bool> {
        Some(self.price_scale_for(pane, target)?.is_inverted())
    }

    pub fn set_price_scale_inverted(&mut self, pane: usize, overlay: bool, inverted: bool) {
        self.invalidate_frame_scene();
        self.set_price_scale_inverted_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
            inverted,
        );
    }

    pub fn set_price_scale_inverted_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        inverted: bool,
    ) {
        self.invalidate_frame_scene();
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.set_invert_scale(inverted);
        }
    }

    pub fn price_scale_margins(&self, pane: usize, overlay: bool) -> Option<(f64, f64)> {
        self.price_scale_margins_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
        )
    }

    pub fn price_scale_margins_for(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<(f64, f64)> {
        let margins = self.price_scale_for(pane, target)?.options().scale_margins;
        Some((margins.top, margins.bottom))
    }

    pub fn set_price_scale_margins(&mut self, pane: usize, overlay: bool, top: f64, bottom: f64) {
        self.invalidate_frame_scene();
        self.set_price_scale_margins_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
            top,
            bottom,
        );
    }

    pub fn set_price_scale_margins_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        top: f64,
        bottom: f64,
    ) {
        self.invalidate_frame_scene();
        if !top.is_finite()
            || !bottom.is_finite()
            || top < 0.0
            || bottom < 0.0
            || top > 1.0
            || bottom > 1.0
            || top + bottom > 1.0
        {
            return;
        }
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.set_scale_margins(top, bottom);
        }
    }

    pub fn price_scale_mode(&self, pane: usize, overlay: bool) -> Option<PriceScaleMode> {
        self.price_scale_mode_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
        )
    }

    pub fn price_scale_mode_for(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<PriceScaleMode> {
        Some(self.price_scale_for(pane, target)?.mode())
    }

    pub fn set_price_scale_mode(&mut self, pane: usize, overlay: bool, mode: PriceScaleMode) {
        self.invalidate_frame_scene();
        self.set_price_scale_mode_for(
            pane,
            if overlay {
                PriceScaleTarget::Overlay
            } else {
                PriceScaleTarget::Right
            },
            mode,
        );
    }

    pub fn set_price_scale_mode_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        mode: PriceScaleMode,
    ) {
        self.invalidate_frame_scene();
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.set_mode(mode);
        }
    }

    /// Whether one live price scale aligns colliding labels.
    pub fn price_scale_align_labels_for(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<bool> {
        Some(self.price_scale_for(pane, target)?.options().align_labels)
    }

    /// Toggle label collision alignment for one live price scale.
    pub fn toggle_price_scale_align_labels(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> bool {
        let Some(scale) = self.price_scale_for_mut(pane, target) else {
            return false;
        };
        scale.set_align_labels(!scale.options().align_labels);
        self.invalidate_frame_all();
        true
    }

    pub fn set_series_price_scale(&mut self, id: SeriesId, target: PriceScaleTarget) {
        let Some(pane_index) = self.series_entry(id).map(|series| series.pane_index) else {
            return;
        };
        if !self
            .panes
            .get(pane_index)
            .is_some_and(|pane| pane.scale(target).is_some())
        {
            return;
        }
        self.invalidate_frame_all();
        let Some(previous) = self
            .series_entry_mut(id)
            .map(|series| std::mem::replace(&mut series.price_scale_target, target))
        else {
            return;
        };
        if previous != target {
            self.adopt_scale_price_format(id);
            // Both the scale the series left and the one it joined refit exactly.
            self.reset_scale_stabilization_at(pane_index, previous);
            self.reset_scale_stabilization_at(pane_index, target);
        }
    }

    /// Merge a snake_case JSON patch of price-scale options into one pane scale (the engine
    /// backing of the TS `priceScale.applyOptions`; unknown keys are ignored gracefully).
    /// Keys: `mode` (0 normal, 1 log, 2 percentage, 3 indexed-to-100), `auto_scale`,
    /// `invert_scale`, `scale_margins` (`{top, bottom}`, each optional), `align_labels`,
    /// `ticks_visible`, `entire_text_only`, `minimum_width`, `text_color` (string, `""` or
    /// `null` clears back to `layout.textColor`), `bold_round_labels`, `tick_mark_density`,
    /// `ensure_edge_tick_marks_visible`, `base_value` and `autoscale_center` (numbers, `null`
    /// clears), and `stable_auto_scale`. Returns false for a malformed patch or an unknown
    /// pane/target.
    pub fn price_scale_apply_options_json(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        json: &str,
    ) -> bool {
        let Ok(serde_json::Value::Object(patch)) = serde_json::from_str::<serde_json::Value>(json)
        else {
            return false;
        };
        let flag = |key: &str| patch.get(key).and_then(serde_json::Value::as_bool);
        let finite = |key: &str| {
            patch
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
        };
        if let Some(visible) = flag("visible")
            && !self.set_price_scale_visible_for(pane, target, visible)
        {
            return false;
        }
        let Some(scale) = self.price_scale_for_mut(pane, target) else {
            return false;
        };
        if let Some(mode) = patch.get("mode").and_then(serde_json::Value::as_u64) {
            scale.set_mode(match mode {
                1 => PriceScaleMode::Logarithmic,
                2 => PriceScaleMode::Percentage,
                3 => PriceScaleMode::IndexedTo100,
                _ => PriceScaleMode::Normal,
            });
        }
        if let Some(auto) = flag("auto_scale") {
            scale.set_auto_scale(auto);
        }
        if let Some(inverted) = flag("invert_scale") {
            scale.set_invert_scale(inverted);
        }
        if let Some(margins) = patch
            .get("scale_margins")
            .and_then(serde_json::Value::as_object)
        {
            let current = scale.options().scale_margins;
            let top = margins
                .get("top")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(current.top);
            let bottom = margins
                .get("bottom")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(current.bottom);
            // Same contract as `set_price_scale_margins_for`: fractions in 0..=1 summing
            // to at most 1; an out-of-contract patch leaves the margins untouched.
            if top.is_finite()
                && bottom.is_finite()
                && top >= 0.0
                && bottom >= 0.0
                && top <= 1.0
                && bottom <= 1.0
                && top + bottom <= 1.0
            {
                scale.set_scale_margins(top, bottom);
            }
        }
        if let Some(align) = flag("align_labels") {
            scale.set_align_labels(align);
        }
        if let Some(visible) = flag("ticks_visible") {
            scale.set_ticks_visible(visible);
        }
        if let Some(entire) = flag("entire_text_only") {
            scale.set_entire_text_only(entire);
        }
        if let Some(width) = finite("minimum_width") {
            scale.set_minimum_width(width);
        }
        if let Some(value) = patch.get("text_color") {
            if value.is_null() {
                scale.set_text_color(None);
            } else if let Some(css) = value.as_str() {
                scale.set_text_color((!css.is_empty()).then(|| css.to_string()));
            }
        }
        if let Some(value) = flag("bold_round_labels") {
            scale.set_bold_round_labels(value);
        }
        if let Some(density) = finite("tick_mark_density") {
            scale.set_tick_mark_density(density);
        }
        if let Some(visible) = flag("ensure_edge_tick_marks_visible") {
            scale.set_ensure_edge_tick_marks_visible(visible);
        }
        // `null` clears; a non-finite (or zero base) value leaves the option untouched.
        let optional_price = |key: &str| match patch.get(key) {
            Some(serde_json::Value::Null) => Some(None),
            Some(value) => value.as_f64().map(Some),
            None => None,
        };
        if let Some(base) = optional_price("base_value") {
            scale.set_base_value(base);
        }
        if let Some(center) = optional_price("autoscale_center") {
            scale.set_autoscale_center(center);
        }
        if let Some(stable) = flag("stable_auto_scale") {
            scale.set_stable_auto_scale(stable);
        }
        // These keys change autoscale inputs, not just axis presentation.
        if [
            "tick_mark_density",
            "ensure_edge_tick_marks_visible",
            "base_value",
            "autoscale_center",
            "stable_auto_scale",
        ]
        .iter()
        .any(|key| patch.contains_key(*key))
        {
            self.invalidate_frame_scene();
        }
        true
    }

    /// reference `tickMarkDensity`: tick label spacing in font heights (finite, positive).
    /// Returns false for an unknown pane/target or an invalid density.
    pub fn set_price_scale_tick_mark_density_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        density: f64,
    ) -> bool {
        self.invalidate_frame_scene();
        self.price_scale_for_mut(pane, target)
            .is_some_and(|scale| scale.set_tick_mark_density(density))
    }

    /// reference `ensureEdgeTickMarksVisible`: rounded boundary ticks and half-font edge padding
    /// while the scale autoscales. Returns false for an unknown pane/target.
    pub fn set_price_scale_ensure_edge_tick_marks_visible_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        visible: bool,
    ) -> bool {
        self.invalidate_frame_scene();
        self.price_scale_for_mut(pane, target)
            .map(|scale| scale.set_ensure_edge_tick_marks_visible(visible))
            .is_some()
    }

    /// Explicit percentage/indexed-to-100 base price for one scale (e.g. the previous close), or
    /// `None` for the first-visible/comparison-anchor base. Drawings on the scale share it.
    /// Returns false for an unknown pane/target or a non-finite/zero base.
    pub fn set_price_scale_base_value_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        base: Option<f64>,
    ) -> bool {
        self.invalidate_frame_scene();
        self.price_scale_for_mut(pane, target)
            .is_some_and(|scale| scale.set_base_value(base))
    }

    /// Center the scale's autoscaled range on a raw price (`center ± max|price − center|`), or
    /// `None` for the plain data range. Returns false for an unknown pane/target or a non-finite
    /// center.
    pub fn set_price_scale_autoscale_center_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        center: Option<f64>,
    ) -> bool {
        self.invalidate_frame_scene();
        self.price_scale_for_mut(pane, target)
            .is_some_and(|scale| scale.set_autoscale_center(center))
    }

    /// Opt one scale into stable autoscale (grow at once, shrink with hysteresis). Returns false
    /// for an unknown pane/target.
    pub fn set_price_scale_stable_auto_scale_for(
        &mut self,
        pane: usize,
        target: PriceScaleTarget,
        stable: bool,
    ) -> bool {
        self.invalidate_frame_scene();
        self.price_scale_for_mut(pane, target)
            .map(|scale| scale.set_stable_auto_scale(stable))
            .is_some()
    }

    /// Restart stable autoscale on one scale (unknown pane/target: no-op).
    pub(crate) fn reset_scale_stabilization_at(&mut self, pane: usize, target: PriceScaleTarget) {
        if let Some(scale) = self.price_scale_for_mut(pane, target) {
            scale.reset_autoscale_stabilization();
        }
    }

    /// Restart stable autoscale on the scale that owns `id`: its contribution changed
    /// structurally (visibility, removal, or pane/scale rebinding), not by a pan.
    pub(crate) fn reset_series_scale_stabilization(&mut self, id: SeriesId) {
        if let Some((pane, target)) = self.series_price_scale(id) {
            self.reset_scale_stabilization_at(pane, target);
        }
    }

    /// A full data replacement restarts stable autoscale on the replaced series' scale and on the
    /// scale of every indicator output rebuilt from it through its price, volume, or turnover
    /// (amount) input (bindings are topological, so one forward pass reaches every downstream
    /// chain).
    pub(crate) fn reset_replaced_series_stabilization(&mut self, id: SeriesId) {
        let mut affected = vec![id];
        for binding in &self.indicators {
            if affected.contains(&binding.source)
                || binding
                    .volume_source
                    .is_some_and(|volume| affected.contains(&volume))
                || binding
                    .amount_source
                    .is_some_and(|amount| affected.contains(&amount))
            {
                affected.extend(binding.outputs.iter().copied());
            }
        }
        for id in affected {
            self.reset_series_scale_stabilization(id);
        }
    }

    /// One pane scale's full options as a snake_case JSON object (reference `priceScale.options()`
    /// shape): `mode`, `auto_scale`, `invert_scale`, `scale_margins`, and the label
    /// cosmetics (`align_labels`, `ticks_visible`, `entire_text_only`, `minimum_width`,
    /// `text_color`, `bold_round_labels`, `tick_mark_density`, `ensure_edge_tick_marks_visible`),
    /// plus `base_value`, `autoscale_center`, `stable_auto_scale`, and `visible`. `None` for an
    /// unknown pane/target.
    pub fn price_scale_options_json(
        &self,
        pane: usize,
        target: PriceScaleTarget,
    ) -> Option<String> {
        let scale = self.price_scale_for(pane, target)?;
        let options = scale.options();
        Some(
            serde_json::json!({
                "mode": match options.mode {
                    PriceScaleMode::Logarithmic => 1,
                    PriceScaleMode::Percentage => 2,
                    PriceScaleMode::IndexedTo100 => 3,
                    PriceScaleMode::Normal => 0,
                },
                "auto_scale": options.auto_scale,
                "invert_scale": options.invert_scale,
                "scale_margins": {
                    "top": options.scale_margins.top,
                    "bottom": options.scale_margins.bottom,
                },
                "align_labels": options.align_labels,
                "ticks_visible": options.ticks_visible,
                "entire_text_only": options.entire_text_only,
                "minimum_width": options.minimum_width,
                "text_color": options.text_color,
                "bold_round_labels": options.bold_round_labels,
                "tick_mark_density": options.tick_mark_density,
                "ensure_edge_tick_marks_visible": options.ensure_edge_tick_marks_visible,
                "base_value": options.base_value,
                "autoscale_center": options.autoscale_center,
                "stable_auto_scale": options.stable_auto_scale,
                "visible": self.price_scale_visible_for(pane, target),
            })
            .to_string(),
        )
    }

    pub fn series_price_scale(&self, id: SeriesId) -> Option<(usize, PriceScaleTarget)> {
        self.series_entry(id)
            .map(|series| (series.pane_index, series.price_scale_target))
    }

    /// First close at or to the right of the visible left edge, matching reference series first-value
    /// selection for percentage/indexed coordinate modes. Whitespace rows are skipped (the reference's
    /// plot list never contains them, so its first-value search lands on a real bar). A custom
    /// series' rows are time-only, so its first value is the host-recorded frame value (the
    /// plugin's `priceValueBuilder` current value of the first visible non-whitespace item —
    /// reference `firstValue` reads the custom plot row's Close slot).
    pub(crate) fn series_base_value(&self, id: SeriesId, visible_from: i64) -> Option<f64> {
        let series = self.series_entry(id)?;
        let first = self.series_first_value(series, visible_from)?;
        // An explicit scale base (e.g. the previous close) replaces the first-visible/anchor base
        // for every source and drawing on that scale. The first visible value still gates whether
        // the source participates at all, exactly as before.
        Some(
            self.price_scale_for(series.pane_index, crate::frame::series_scale_target(series))
                .and_then(|scale| scale.options().base_value)
                .unwrap_or(first),
        )
    }

    fn series_first_value(&self, series: &crate::SeriesEntry, visible_from: i64) -> Option<f64> {
        let id = series.id;
        if series.kind == SeriesKind::Custom {
            return series
                .custom_frame
                .first_value
                .filter(|value| value.is_finite());
        }
        let plot = self.data.plot(id);
        let row = self
            .comparison_anchor
            .and_then(|time| self.axis_index_for_time(time))
            .and_then(|index| plot.search(index as i64, MismatchDirection::NearestLeft))
            .filter(|&row| !plot.is_whitespace_row(row))
            .or_else(|| plot.first_non_whitespace_row(visible_from))?;
        let value = self
            .heikin_ashi_row(id, row)
            .map(|values| values[3])
            .unwrap_or_else(|| plot.value_at(row, PlotValueIndex::Close));
        value.is_finite().then_some(value)
    }

    pub(crate) fn visible_series_base_value(&self, id: SeriesId) -> Option<f64> {
        let from = self.time_scale.visible_strict_range()?.left();
        self.series_base_value(id, from)
    }

    /// Set or clear the chart-wide comparison anchor. The timestamp must be a finite integral
    /// chart time (UTC seconds in the public API); a non-integral value is rejected so all hosts
    /// resolve the same merged-time identity. Returns whether the request changed the anchor.
    pub fn set_comparison_anchor(&mut self, time: Option<f64>) -> bool {
        let next = match time {
            None => None,
            Some(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && value >= i64::MIN as f64
                    && value <= i64::MAX as f64 =>
            {
                Some(value as i64)
            }
            Some(_) => return false,
        };
        if self.comparison_anchor == next {
            return false;
        }
        self.comparison_anchor = next;
        self.invalidate_frame_all();
        true
    }

    /// Current chart-wide comparison anchor, if configured.
    pub fn comparison_anchor(&self) -> Option<i64> {
        self.comparison_anchor
    }

    /// Resolve the bounded legend values for all live series using one shared anchor. Missing
    /// anchor rows stay explicit as `None`; no neighboring series or host-owned cache is used.
    pub fn comparison_legend_snapshot(&self) -> Vec<crate::ComparisonLegendEntry> {
        let anchor_time = self.comparison_anchor;
        let anchor_index = anchor_time.and_then(|time| self.axis_index_for_time(time));
        self.series
            .iter()
            .filter(|series| !series.removed && series.visible)
            .filter_map(|series| {
                let plot = self.data.plot(series.id);
                let anchor_value = anchor_index
                    .and_then(|index| plot.search(index as i64, MismatchDirection::NearestLeft))
                    .filter(|&row| !plot.is_whitespace_row(row))
                    .map(|row| plot.value_at(row, PlotValueIndex::Close))
                    .filter(|value| value.is_finite());
                let latest_row = plot.last_non_whitespace_row(i64::MAX)?;
                let latest_value = plot.value_at(latest_row, PlotValueIndex::Close);
                if !latest_value.is_finite() {
                    return None;
                }
                let latest_time = plot
                    .index_at(latest_row)
                    .and_then(|index| self.axis_time_key_at(index as usize));
                let change = anchor_value.map(|value| latest_value - value);
                let percent_change = anchor_value
                    .filter(|value| *value != 0.0)
                    .map(|value| (latest_value - value) / value * 100.0);
                Some(crate::ComparisonLegendEntry {
                    series_id: series.id,
                    title: series.title.clone(),
                    anchor_time,
                    anchor_value,
                    latest_time,
                    latest_value: Some(latest_value),
                    change,
                    percent_change,
                })
            })
            .collect()
    }
}
