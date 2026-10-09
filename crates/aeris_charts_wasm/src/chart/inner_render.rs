//! `ChartInner` rendering: WebGPU/Canvas2D pane execution, axis overlay painting, backend
//! failover, and browser text measurement.

use aeris_charts_render::draw_list::Prim;

use super::*;

impl ChartInner {
    /// Reports the active pane backend for diagnostics and runtime-matrix tests.
    pub fn backend_kind(&self) -> String {
        if self.gfx.is_some() {
            "webgpu".into()
        } else {
            "canvas2d".into()
        }
    }

    pub fn render(&mut self) -> Result<(), JsValue> {
        // `frame_stats().cpu_ms` covers this whole function: layout, axis-frame construction,
        // engine frame build, plugin passes, and command encoding — the host-side CPU cost of
        // producing a frame. Two clock reads per frame; the record itself is fixed-size.
        let frame_start = self.clock.as_ref().map(|clock| clock.now());
        self.telemetry.reset_canvas2d_ops();
        self.telemetry.set_gpu_resources(0, 0, 0);
        self.telemetry.set_browser_rebuilds(0, 0);
        let outcome = self.render_inner();
        if let (Some(clock), Some(start)) = (self.clock.as_ref(), frame_start) {
            self.telemetry.set_cpu_ms(clock.now() - start);
        }
        outcome
    }

    fn render_inner(&mut self) -> Result<(), JsValue> {
        // Series primitives (plugin platform Phase C-b): pull this frame's autoscale
        // contributions from the plugin hooks before any layout/autoscale pass runs, so the
        // axis-width negotiation, axis frame, and pane frame all see the merged ranges.
        self.collect_series_primitive_autoscale();
        // Custom series (Phase C-c): same pre-layout collection point — the visible items'
        // price values become autoscale contributions and the engine's custom frame values.
        self.collect_custom_series_autoscale();
        let plugin_active = !self.primitives.is_empty()
            || !self.series_primitives.is_empty()
            || !self.custom_series.is_empty();
        if plugin_active {
            // Plugins append transient labels to `axis_frame`; rebuild its engine-owned base on
            // every conservative plugin frame so labels neither duplicate nor survive detach.
            self.engine.invalidate_axis_frame();
        }

        // Feed the engine clock for the candle-close countdown labels: the host-pinned value
        // when `set_now_seconds` installed one (the package's 1s countdown timer), else the
        // browser's system time — the engine itself is headless and owns no clock. This must
        // happen before layout so the first rendered countdown participates in width negotiation.
        let now = self
            .now_override
            .unwrap_or_else(|| js_sys::Date::now() / 1000.0);
        self.engine.set_now_seconds(now);

        // The engine settles layout, autoscale, pane layers, and base axis labels together.
        // Plugin labels enter after this call and before axis primitive lowering.
        let layout = self.opts().layout;
        let font_family = layout.font_family;
        let axis_size = self.engine.axis_font_size();
        let countdown_size = self.engine.countdown_font_size();
        let axis_ctx = &self.axis_ctx;
        let dpr = self.dpr;
        let prepared = self.engine.prepare_financial_frame_with_measure(
            aeris_charts_engine::FinancialFrameRequest {
                width: self.css_width,
                height: self.css_height,
                dpr,
                force_layout: false,
                allow_axis_shrink: false,
                force_frame: plugin_active,
                force_axis: self.axis_dirty,
                layout_only: false,
                fit_content: false,
                frame: &mut self.frame,
                axis_frame: Some(&mut self.axis_frame),
                axis_primitives: None,
            },
            |text, bold| measure_text_ctx(axis_ctx, dpr, &font_family, axis_size, bold, text),
            |text, bold| measure_text_ctx(axis_ctx, dpr, &font_family, countdown_size, bold, text),
        );
        self.telemetry.set_rebuilds(self.engine.frame_build_stats());
        if prepared.axis_rebuilt {
            // Colors and chrome options may change without label geometry changing.
            self.axis_dirty = true;
        }

        // Pane primitives record their geometry and text in the ordered pane layers after the
        // engine frame is settled and before either backend consumes it.
        self.run_pane_primitives();
        // Series primitives (Phase C-b): same pass, bound to each owning series' scale.
        self.run_series_primitives();
        // Custom series (Phase C-c): plugin renders splice into each pane's `main` layer at
        // the series' paint-order marks (same command-recording model).
        self.run_custom_series();
        if !self.primitives.is_empty()
            || !self.series_primitives.is_empty()
            || !self.custom_series.is_empty()
        {
            self.axis_dirty = true;
        }
        // Axis labels contributed by primitives are now complete; convert the whole top layer once
        // and feed it to whichever backend executes this frame.
        let axis_rebuilt = self.axis_dirty;
        self.build_axis_prims();
        let text_rasterizations_before = self
            .text_runs
            .as_ref()
            .map_or(0, TextRunStore::rasterizations);

        if self
            .gfx
            .as_ref()
            .is_some_and(|gfx| gfx.device_lost.load(Ordering::Acquire))
        {
            self.activate_canvas2d("device_lost", "WebGPU device was lost");
        }

        let bg = resolved_surface_color(&self.opts().layout.background.color);
        // Arm GPU timestamp collection on the first frame after the host reads `frame_stats()`.
        // `GpuTimer::new` is a feature-flag check plus (once) a query set, so an unsupported
        // device just keeps answering `None` and `gpu_ms` stays null.
        if self.telemetry.stats_requested()
            && let Some(gfx) = self.gfx.as_mut()
            && gfx.timer.is_none()
        {
            gfx.timer = GpuTimer::new(&gfx.shared.device, &gfx.shared.queue);
        }

        let pane_outcome = if self.gfx.is_some() {
            let engine_frame = &self.frame;
            let pane_count = engine_frame.panes.len();
            let pane_group_count = if plugin_active {
                pane_count
            } else {
                (0..pane_count)
                    .map(|pane| {
                        6 + self.engine.frame_series_segments(pane).len()
                            + self.engine.frame_drawing_segments(pane).len()
                    })
                    .sum()
            };
            self.gpu_groups
                .resize_with(pane_group_count + 1, DrawGroup::default);
            self.gpu_groups.truncate(pane_group_count + 1);
            let Some(gfx) = self.gfx.as_mut() else {
                return Err(JsValue::from_str("WebGPU state disappeared mid-render"));
            };
            let shared = Rc::clone(&gfx.shared);
            let renderers = Rc::clone(&gfx.renderers);
            let text_runs = &mut self.text_runs;
            let mut atlas = shared.atlas.borrow_mut();
            let mut image_atlas = shared.image_atlas.borrow_mut();
            atlas.begin_frame();
            image_atlas.begin_frame();
            let atlas_changed = self.gpu_atlas_epoch != atlas.epoch();
            let image_atlas_changed = self.gpu_image_atlas_epoch != image_atlas.epoch();
            self.gpu_atlas_epoch = atlas.epoch();
            self.gpu_image_atlas_epoch = image_atlas.epoch();
            if !atlas_changed
                && self
                    .gpu_groups
                    .iter()
                    .any(|group| !group.tex_quads.is_empty())
            {
                atlas.protect_retained_frame_slots();
            }
            if !image_atlas_changed
                && self
                    .gpu_groups
                    .iter()
                    .any(|group| !group.image_quads.is_empty())
            {
                image_atlas.protect_retained_frame_slots();
            }
            let queue = &shared.queue;
            let mut lowering = GroupLowering {
                text_runs,
                atlas: &mut atlas,
                image_atlas: &mut image_atlas,
                queue,
            };
            // An unchanged group that already dropped a text run would drop it again, so the
            // frame is routed to Canvas2D before any WebGPU tessellation. Every other retained
            // group keeps its geometry for the frame after the offending text changes or goes.
            let known_text_failure = !atlas_changed
                && !image_atlas_changed
                && (is_known_text_failure(
                    &self.gpu_groups[pane_group_count],
                    AXIS_GROUP_KEY,
                    self.axis_revision,
                    None,
                ) || (!plugin_active && {
                    let groups = &self.gpu_groups;
                    let mut known = false;
                    for_each_retained_group(&self.engine, engine_frame, |index, slot| {
                        known |= is_known_text_failure(
                            &groups[index],
                            slot.key,
                            slot.source_revision,
                            slot.scissor,
                        );
                    });
                    known
                }));
            let mut text_failed = known_text_failure;
            if !known_text_failure && plugin_active {
                for (pane, pane_frame) in engine_frame.panes.iter().enumerate() {
                    let group = &mut self.gpu_groups[pane];
                    group.key = 0x1000_0000 | pane as u64;
                    group.scissor = Some(pane_frame.scissor);
                    group.clear();
                    for layer in [&pane_frame.under, &pane_frame.main, &pane_frame.top_prims] {
                        text_failed |= lowering.lower(group, layer, &pane_frame.points);
                    }
                }
            } else if !known_text_failure {
                let groups = &mut self.gpu_groups;
                for_each_retained_group(&self.engine, engine_frame, |index, slot| {
                    let group = &mut groups[index];
                    if !atlas_changed
                        && !image_atlas_changed
                        && group.key == slot.key
                        && group.source_revision == slot.source_revision
                        && group.scissor == slot.scissor
                    {
                        return;
                    }
                    group.scissor = slot.scissor;
                    group.rebuild(slot.key, slot.source_revision);
                    if lowering.lower(group, slot.prims, slot.points) {
                        group.key ^= TEXT_FAILED_KEY_BIT;
                        text_failed = true;
                    }
                });
            }
            // Final unscissored top-layer group: watermark, axis chrome and axis/crosshair labels.
            // It is submitted in this same pass after every pane group, so no engine Canvas2D paint
            // follows a WebGPU frame.
            let axis_group = &mut self.gpu_groups[pane_group_count];
            if !known_text_failure
                && (atlas_changed
                    || image_atlas_changed
                    || axis_group.key != AXIS_GROUP_KEY
                    || axis_group.source_revision != self.axis_revision)
            {
                axis_group.scissor = None;
                axis_group.rebuild(AXIS_GROUP_KEY, self.axis_revision);
                if lowering.lower(axis_group, &self.axis_prims, &[]) {
                    axis_group.key ^= TEXT_FAILED_KEY_BIT;
                    text_failed = true;
                }
            }
            let atlas_valid = atlas.frame_valid() && image_atlas.frame_valid() && !text_failed;
            drop(atlas);
            drop(image_atlas);
            if !atlas_valid {
                PaneRenderOutcome::Canvas2d
            } else {
                let groups = &self.gpu_groups[..];
                gfx.msaa.ensure(
                    &shared.device,
                    gfx.config.format,
                    gfx.config.width,
                    gfx.config.height,
                );

                let acquired = match gfx.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(Some(frame)),
                    error => match surface_error_action(&error) {
                        SurfaceErrorAction::Reconfigure => {
                            // Resize and suspend/resume can invalidate only the swapchain. Reconfigure
                            // and retry once; if that fails, the warm Canvas2D pane takes over.
                            gfx.surface.configure(&shared.device, &gfx.config);
                            match gfx.surface.get_current_texture() {
                                wgpu::CurrentSurfaceTexture::Success(frame)
                                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(Some(frame)),
                                retry_error
                                    if surface_error_action(&retry_error)
                                        == SurfaceErrorAction::SkipFrame =>
                                {
                                    Ok(None)
                                }
                                _ => Err("surface remained unavailable after reconfiguration"),
                            }
                        }
                        SurfaceErrorAction::SkipFrame => Ok(None),
                        SurfaceErrorAction::Fallback => Err("surface validation failed"),
                    },
                };

                match acquired {
                    Ok(Some(frame)) => {
                        let view = frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        let bg_clear = wgpu::Color {
                            r: f64::from(bg.r()) * f64::from(bg.a()) / (255.0 * 255.0),
                            g: f64::from(bg.g()) * f64::from(bg.a()) / (255.0 * 255.0),
                            b: f64::from(bg.b()) * f64::from(bg.a()) / (255.0 * 255.0),
                            a: f64::from(bg.a()) / 255.0,
                        };
                        let resources_before = gfx.frame_resources.stats();
                        let draw_calls = render_frame(
                            &shared.device,
                            &shared.queue,
                            gfx.msaa.view(),
                            &view,
                            gfx.config.width,
                            gfx.config.height,
                            bg_clear,
                            &renderers.quad,
                            &renderers.tex,
                            &renderers.rotated_tex,
                            &renderers.image,
                            &renderers.tri,
                            groups,
                            &mut gfx.frame_resources,
                            gfx.timer.as_ref(),
                        );
                        let resources_after = gfx.frame_resources.stats();
                        self.telemetry.set_gpu_resources(
                            resources_after.allocations - resources_before.allocations,
                            resources_after.write_calls - resources_before.write_calls,
                            resources_after.uploaded_bytes - resources_before.uploaded_bytes,
                        );
                        self.telemetry.set_draw_calls(draw_calls);
                        shared.queue.present(frame);
                        PaneRenderOutcome::Presented
                    }
                    Ok(None) => PaneRenderOutcome::Timeout,
                    Err(error) => PaneRenderOutcome::Fallback(format!(
                        "WebGPU surface acquisition failed after recovery: {error}"
                    )),
                }
            }
        } else {
            PaneRenderOutcome::Canvas2d
        };

        let text_rasterizations_after = self
            .text_runs
            .as_ref()
            .map_or(0, TextRunStore::rasterizations);
        self.telemetry.set_browser_rebuilds(
            u64::from(axis_rebuilt),
            text_rasterizations_after.saturating_sub(text_rasterizations_before),
        );

        match pane_outcome {
            PaneRenderOutcome::Presented => {
                set_backend_visibility(self.gpu_pane.as_ref(), self.fallback_pane.as_ref(), true);
            }
            PaneRenderOutcome::Timeout => {
                // Keep the last complete frame. The next animation/input repaint retries.
                // This is exactly `frame_stats().dropped_frames`: encoded but never presented.
                self.telemetry.count_dropped();
                return Ok(());
            }
            PaneRenderOutcome::Fallback(reason) => {
                self.activate_canvas2d("surface_acquisition_failed", &reason);
                self.render_canvas2d()?;
            }
            PaneRenderOutcome::Canvas2d => {
                self.render_canvas2d()?;
                set_backend_visibility(self.gpu_pane.as_ref(), self.fallback_pane.as_ref(), false);
            }
        }

        self.telemetry.count_presented();
        Ok(())
    }

    // ---- Shared axis/top-layer frame ----

    /// Convert the engine-owned [`AxisFrame`] plus watermark into the shared backend-neutral
    /// axis/top layer. The engine owns this policy; this browser host contributes only its native
    /// Canvas text ink metric.
    fn build_axis_prims(&mut self) {
        if !self.axis_dirty {
            return;
        }
        self.engine
            .build_axis_primitives_into(&self.axis_frame, &mut self.axis_prims);
        self.axis_revision = self.axis_revision.wrapping_add(1).max(1);
        self.axis_dirty = false;
    }

    /// Permanently switch this chart instance to its already-initialized Canvas2D pane.
    fn activate_canvas2d(&mut self, stable_reason: &'static str, detail: &str) {
        if self.gfx.take().is_some() {
            set_backend_visibility(self.gpu_pane.as_ref(), self.fallback_pane.as_ref(), false);
            self.backend_status
                .runtime_fallback(stable_reason, detail.to_string());
            warn_backend_fallback(&self.backend_status);
        }
    }

    /// Execute the exact same retained frame consumed by WebGPU through Canvas2D.
    pub(super) fn render_canvas2d(&self) -> Result<(), JsValue> {
        self.render_canvas2d_with_axis(true)
    }

    /// Execute the retained pane frame through Canvas2D, optionally including the shared
    /// watermark/axis top layer. Screenshot capture passes `false` to preserve the established
    /// `take_screenshot(add_top_layer = false)` behavior now that axes no longer live on the
    /// transparent overlay canvas.
    pub(super) fn render_canvas2d_with_axis(&self, include_axis: bool) -> Result<(), JsValue> {
        let ctx = &self.pane_ctx;
        let width = self.bitmap_w as f64;
        let height = self.bitmap_h as f64;
        ctx.clear_rect(0.0, 0.0, width, height);
        let bg = self.opts().layout.background.color;
        ctx.set_fill_style_str(&bg);
        ctx.fill_rect(0.0, 0.0, width, height);
        let mut image_store = self.canvas_images.borrow_mut();
        let mut target = crate::canvas2d_target::WasmCanvas2d::with_images(ctx, &mut image_store);
        let viewport = CanvasViewport {
            width: width as f32,
            height: height as f32,
        };
        for pane in &self.frame.panes {
            target.save();
            let [x, y, w, h] = pane.scissor;
            target.clip_rect(x as f32, y as f32, w as f32, h as f32);
            execute_canvas2d(&pane.under, &pane.points, &mut target, viewport);
            execute_canvas2d(&pane.main, &pane.points, &mut target, viewport);
            execute_canvas2d(&pane.top_prims, &pane.points, &mut target, viewport);
            target.restore();
        }
        if include_axis {
            execute_canvas2d(&self.axis_prims, &[], &mut target, viewport);
        }
        // The `clear_rect` + background `fill_rect` above, plus every executed prim.
        self.telemetry.add_canvas2d_ops(2 + target.ops());
        Ok(())
    }
}

const AXIS_GROUP_KEY: u64 = u64::MAX;

/// Toggled into a retained group's key when lowering it dropped a visible text run. No live slot
/// key uses this bit (the axis key is all ones and toggles to a value no pane slot can reach), so
/// a marked group never matches as clean and is rebuilt as soon as its source changes.
const TEXT_FAILED_KEY_BIT: u64 = 1 << 63;

fn is_known_text_failure(
    group: &DrawGroup,
    key: u64,
    source_revision: u64,
    scissor: Option<[u32; 4]>,
) -> bool {
    group.key == key ^ TEXT_FAILED_KEY_BIT
        && group.source_revision == source_revision
        && group.scissor == scissor
}

/// One retained WebGPU pane group: its stable slot key, the engine revision of its source
/// range, its scissor, and the primitives it lowers.
struct RetainedGroup<'a> {
    key: u64,
    source_revision: u64,
    scissor: Option<[u32; 4]>,
    prims: &'a [Prim],
    points: &'a [[f32; 2]],
}

/// Visits every retained pane group in submission order with its `gpu_groups` index. The
/// failure pre-check and the build pass share this walk so they always agree on slot identity.
fn for_each_retained_group<'a>(
    engine: &'a ChartEngine,
    frame: &'a aeris_charts_engine::ChartFrame,
    mut visit: impl FnMut(usize, RetainedGroup<'a>),
) {
    let mut index = 0;
    let mut emit = |key: u64,
                    source_revision: u64,
                    scissor: [u32; 4],
                    prims: &'a [Prim],
                    points: &'a [[f32; 2]]| {
        visit(
            index,
            RetainedGroup {
                key,
                source_revision,
                scissor: Some(scissor),
                prims,
                points,
            },
        );
        index += 1;
    };
    for (pane, pane_frame) in frame.panes.iter().enumerate() {
        let segments = engine.frame_pane_segments(pane).unwrap_or_default();
        let main = &pane_frame.main[..];
        let points = &pane_frame.points[..];
        let scissor = pane_frame.scissor;
        let range = |start: usize, end: usize| &main[start.min(main.len())..end.min(main.len())];
        emit(
            0x2000_0000 | (pane as u64) << 4,
            segments.under_revision,
            scissor,
            &pane_frame.under[..segments.under_end.min(pane_frame.under.len())],
            points,
        );
        // Chart content merged in paint order (ordering.rs): idle indicators, idle drawings,
        // ordinary series, active series/drawings/previews, chrome. Series and drawing segments
        // both carry out.main ranges; merging by start preserves pane-local order so WebGPU
        // blends like Canvas2D. Ordering-only promotion swaps group order (key mismatch
        // re-uploads moved groups) without rebuilding retained geometry.
        let series_segs = engine.frame_series_segments(pane);
        let drawing_segs = engine.frame_drawing_segments(pane);
        let (mut si, mut di) = (0usize, 0usize);
        loop {
            let take_series = match (series_segs.get(si), drawing_segs.get(di)) {
                (Some(s), Some(d)) => s.start <= d.start,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if take_series {
                let series = &series_segs[si];
                si += 1;
                let key = match series.series_id {
                    Some(id) => 0x3000_0000 | (pane as u64) << 32 | u64::from(id),
                    None => 0x4000_0000 | pane as u64,
                };
                emit(
                    key,
                    series.revision,
                    scissor,
                    range(series.start, series.end),
                    points,
                );
            } else {
                let drawing = &drawing_segs[di];
                di += 1;
                let key = match drawing.drawing_id {
                    Some(id) => 0x5000_0000 | (pane as u64) << 32 | u64::from(id),
                    None => 0x6000_0000 | pane as u64,
                };
                emit(
                    key,
                    drawing.revision,
                    scissor,
                    range(drawing.start, drawing.end),
                    points,
                );
            }
        }
        emit(
            0x2000_0005 | (pane as u64) << 4,
            segments.trading_revision,
            scissor,
            range(segments.series_end, segments.trading_regions_end),
            points,
        );
        emit(
            0x2000_0002 | (pane as u64) << 4,
            segments.drawings_revision,
            scissor,
            range(segments.trading_regions_end, segments.drawings_end),
            points,
        );
        emit(
            0x2000_0006 | (pane as u64) << 4,
            segments.trading_revision,
            scissor,
            range(segments.drawings_end, segments.trading_end),
            points,
        );
        emit(
            0x2000_0003 | (pane as u64) << 4,
            segments.overlay_revision,
            scissor,
            range(segments.trading_end, segments.overlay_end),
            points,
        );
        emit(
            0x2000_0004 | (pane as u64) << 4,
            segments.top_revision,
            scissor,
            &pane_frame.top_prims,
            points,
        );
    }
}

/// Per-frame text/image resolution state shared by every group lowered in one WebGPU frame.
struct GroupLowering<'f> {
    text_runs: &'f mut Option<TextRunStore>,
    atlas: &'f mut LabelAtlas,
    image_atlas: &'f mut LabelAtlas,
    queue: &'f wgpu::Queue,
}

impl GroupLowering<'_> {
    /// Appends `prims` to `group`; returns whether a visible text run could not be resolved.
    fn lower(&mut self, group: &mut DrawGroup, prims: &[Prim], points: &[[f32; 2]]) -> bool {
        let mut failed = false;
        let Self {
            text_runs,
            atlas,
            image_atlas,
            queue,
        } = self;
        let mut resolve_text = |prim: &Prim| {
            let instance = text_runs
                .as_mut()
                .and_then(|runs| runs.resolve(atlas, queue, prim));
            if instance.is_none()
                && matches!(prim, Prim::Text { text, .. } | Prim::RotatedText { text, .. } if !text.trim().is_empty())
            {
                failed = true;
            }
            instance
        };
        let mut resolve_image = |prim: &Prim| super::image_runs::resolve(image_atlas, queue, prim);
        prims_to_group(prims, points, group, &mut resolve_text, &mut resolve_image);
        failed
    }
}

fn resolved_surface_color(css: &str) -> Color {
    let fallback = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
    Color::parse_css(css).unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn malformed_surface_css_falls_back_to_the_canonical_aeris_surface() {
        let expected = aeris_charts_core::style::DEFAULT_SURFACE_RGB;
        assert_eq!(
            resolved_surface_color("not-a-color"),
            Color::rgb(expected.0, expected.1, expected.2)
        );
    }
}

pub(super) fn measure_text_ctx(
    ctx: &CanvasRenderingContext2d,
    dpr: f64,
    font_family: &str,
    font_size: f64,
    bold: bool,
    text: &str,
) -> f64 {
    let weight = if bold { 700 } else { 400 };
    ctx.set_font(&format!("{weight} {}px {font_family}", font_size * dpr));
    ctx.measure_text(text).map(|m| m.width()).unwrap_or(0.0) / dpr
}
