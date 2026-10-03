/**
 * Pane primitives (plugin platform Phase C-a), independently designed after studying the public
 * primitive APIs and observable behavior of established financial-chart libraries.
 *
 * Locked design divergence: a primitive
 * never touches a canvas. Its view renderers record backend-neutral draw commands through the
 * context below; the host folds them into the same `Prim` IR the WebGPU and Canvas2D backends
 * both consume, so plugin output is pixel-identical across backends and z-orders between the
 * engine's own layers (`bottom` behind the series, `top` above the crosshair).
 */

/** Where in the pane's layer stack a view paints (reference `PrimitivePaneViewZOrder`). */
export type primitive_z_order = "bottom" | "normal" | "top";

/**
 * reference `LineStyle` wire values: 0 solid, 1 dotted, 2 dashed, 3 large-dashed, 4 sparse-dotted.
 * Every backend paints the same dashes: 1 and 4 are the sparse-dot pattern (`width` on, `4 * width`
 * off) and 2 and 3 are the long-dash pattern (`6 * width` on, `6 * width` off), measured in bitmap
 * px and started at the first point (`polyline`) or at `x1`/`y1` (`hline`/`vline`).
 * A dashed `polyline` needs `width >= 0.5` bitmap px and is skipped with a console warning below it,
 * and one whose visible path would lower to more than 4096 dashes (a dense zigzag through the
 * pane) is drawn solid with a console warning, so a frame never pays for the path's length.
 */
export type primitive_line_style = 0 | 1 | 2 | 3 | 4;

/**
 * Draw context handed to a {@link primitive_pane_view} renderer. All coordinates are absolute
 * bitmap px of the whole chart (x from its left edge, y from its top — pane offsets included),
 * the same space the engines' geometry uses; `pane_left`/`pane_top` give the pane's offset and
 * `dpr` the nominal device pixel ratio. The converters resolve against the settled post-layout
 * scales (the pane's right price scale by default) and return `null` when a value falls off the
 * scale — mirroring the reference's `timeToCoordinate`/`priceToCoordinate` nullability.
 *
 * `price_to_y`/`time_to_x`/`logical_to_x` return this bitmap space (device px, `x` including
 * `pane_left`), not the CSS-px space of the chart and series `*_to_coordinate` converters, whose
 * `x` starts at the plot-area left edge: subtract `pane_left` from an `x` and divide by `dpr`
 * before mixing the two.
 *
 * The context is valid only for the duration of the synchronous `renderer` call.
 */
export interface primitive_draw_context {
  /** Pane bitmap width in px. */
  readonly pane_width: number;
  /** Pane bitmap height in px. */
  readonly pane_height: number;
  /** Pane offset x in absolute bitmap px (0 unless a left price axis is visible). */
  readonly pane_left: number;
  /** Pane offset y in absolute bitmap px. */
  readonly pane_top: number;
  /** The chart's nominal device pixel ratio. */
  readonly dpr: number;
  /** Bitmap y for a price on the pane's `target` scale (default `"right"`); `null` off-scale. */
  price_to_y(price: number, target?: "left" | "right"): number | null;
  /** Bitmap x for an exact bar timestamp (UTC seconds); `null` when not a bar (reference no-snap). */
  time_to_x(time: number): number | null;
  /** Bitmap x for a (possibly fractional) logical bar index; `null` when there is no data. */
  logical_to_x(index: number): number | null;
  /** Filled rect (integer-snapped, Canvas2D `fillRect` semantics). */
  rect(x: number, y: number, w: number, h: number, color: string): void;
  /** Hollow frame filled inside the rect (Canvas2D `fillRectInnerBorder` semantics). */
  rect_frame(x: number, y: number, w: number, h: number, color: string, line_width: number): void;
  /** Horizontal line, integer-snapped, with an reference line style. */
  hline(y: number, x1: number, x2: number, color: string, width: number, style: primitive_line_style): void;
  /** Vertical line, integer-snapped, with an reference line style. */
  vline(x: number, y1: number, y2: number, color: string, width: number, style: primitive_line_style): void;
  /** Anti-aliased polyline; `points` is a flat `[x0, y0, x1, y1, ...]` array. */
  polyline(points: readonly number[], color: string, width: number, style: primitive_line_style): void;
  /** Vertical-gradient fill between a polyline and `base_y`; `points` flat as in `polyline`. */
  area_fill(points: readonly number[], base_y: number, top_color: string, bottom_color: string): void;
  /** Filled disc with an optional border ring. */
  circle(x: number, y: number, r: number, fill_color: string, border_color?: string, border_width?: number): void;
  /** Rounded-corner filled rect (one radius for all four corners). */
  round_rect(x: number, y: number, w: number, h: number, r: number, color: string): void;
  /** Filled triangle. */
  triangle(x1: number, y1: number, x2: number, y2: number, x3: number, y3: number, color: string): void;
  /**
   * In-pane text, painted at the command's position in the layer order on BOTH backends
   * (Canvas2D `fillText` directly; WebGPU via a browser-rasterized atlas quad of the same
   * run — pixel-identical glyphs by construction). The anchor is (x, y) in absolute bitmap
   * px: x is the aligned edge (`align`, default `"left"`), y the vertical center of the run
   * (Canvas `textBaseline: "middle"`, the axis-label convention). `options.size` is the glyph
   * size in bitmap px (default: `layout.fontSize × dpr`), `options.font` the font family
   * (default: `layout.fontFamily`), `options.color` any CSS color (default:
   * `layout.textColor`), `options.weight` the numeric CSS font weight 100–900 (default 400;
   * `options.bold` is the legacy shorthand for 700), and `options.italic` the style. For
   * text in the shared top pane layer use {@link pane_primitive.text_views}.
   */
  text(
    x: number,
    y: number,
    text: string,
    options?: {
      color?: string;
      size?: number;
      font?: string;
      align?: "left" | "center" | "right";
      bold?: boolean;
      weight?: number;
      italic?: boolean;
    },
  ): void;
}

/**
 * One paintable view of a primitive (reference `IPanePrimitivePaneView`). `renderer` runs once per
 * frame per view; keep it cheap.
 */
export interface primitive_pane_view {
  /** Layer to paint into (default `"normal"`): `bottom` behind the series, `top` above all. */
  z_order?: primitive_z_order;
  /**
   * Record this view's drawing for the current frame. Synchronous; do not stash `ctx`.
   * Runs while the chart is mid-render: do not call chart/series/scale APIs from inside it
   * (the context converters are the read path); capture any other inputs from outside.
   */
  renderer(ctx: primitive_draw_context): void;
}

/**
 * A primitive's hit-test result (reference `PrimitiveHoveredItem`, reduced to Aeris's arbitration
 * model: the host owns z-ordering and series-vs-primitive precedence, so the reference's `distance`,
 * `hitTestPriority`, `itemType`, and `isBackground` fields are not modeled — within a layer,
 * the first hit in paint order wins).
 */
export interface primitive_hit_result {
  /** Identifier reported as `mouse_event_params.hovered_object_id` while the hit holds. */
  external_id?: string;
  /** CSS cursor applied to the chart while the hit holds (reference `cursorStyle`). */
  cursor_style?: string;
  /**
   * The layer the hit belongs to (default `"normal"`). A `"top"` hit always beats the series
   * hit tests; a `"normal"` hit beats its own series' built-in hit and every series below
   * it; a `"bottom"` hit only survives when no series is hit (reference pane-hit-test.ts).
   */
  z_order?: primitive_z_order;
}

/**
 * A boxed axis label descriptor (cf. reference `ISeriesPrimitiveAxisView`). `coordinate` is media px
 * from the pane's top edge (price axis) or the pane's left edge (time axis): unlike the chart's
 * public converters, which return chart-content `y` from the top of the stacked panes, it is
 * pane-local (a series primitive's `price` wins over it and is converted on the series' scale).
 * `background_color` (or `color` as a shorthand) fills the box; omitted `text_color` automatically
 * selects black or white for contrast with the effective background.
 * Extension over reference: reference exposes axis views only on series primitives; Aeris
 * accepts them on pane primitives too (painted on the pane's right scale / the time strip).
 */
export interface primitive_axis_label {
  text: string;
  coordinate: number;
  color?: string;
  background_color?: string;
  text_color?: string;
}

/**
 * An in-pane text draw registered through {@link pane_primitive.text_views} /
 * {@link series_primitive.text_views} (plugin platform Phase 3.5). It is painted through the
 * shared top pane layer on every backend and clipped strictly to its owning pane,
 * so it cannot cover price/time-axis chrome. For text at a specific layer position between the
 * engine's own primitives (for example behind the series), use
 * {@link primitive_draw_context.text} instead.
 *
 * `x`/`y` are absolute bitmap px (the draw context's coordinate space); the shared frame applies
 * the owning-pane clip. `color` is a CSS color resolved by Canvas and encoded as shared RGBA.
 * `font` is a CSS font shorthand for style, weight, pixel size and family, and wins over `size`/`font_family`/
 * `bold`; without it the host composes `{bold } {size}px {family}` defaulting to the chart's
 * `layout.fontSize`/`layout.fontFamily` — the same string the engine's own axis labels use.
 */
export interface primitive_text_view {
  text: string;
  x: number;
  y: number;
  color?: string;
  font?: string;
  /** Glyph size in CSS px (default: the chart's `layout.fontSize`). */
  size?: number;
  font_family?: string;
  bold?: boolean;
  /** Canvas `textAlign` keyword (default `"left"`). */
  align?: "left" | "center" | "right";
  /** Canvas `textBaseline` keyword (default `"alphabetic"`). */
  baseline?: "top" | "middle" | "bottom" | "alphabetic";
}

/**
 * The layout state handed to {@link pane_primitive.text_views} / {@link series_primitive.text_views}.
 * Plugins cannot call chart APIs from render hooks, so everything a text layout needs is passed
 * in: the pane's bitmap dimensions (same space as the draw context), the frame's exact
 * horizontal/vertical pixel ratios (`hpr`/`vpr` — media px = bitmap px / ratio), the nominal
 * `dpr`, and the chart's layout font.
 */
export interface primitive_text_context {
  readonly pane_width: number;
  readonly pane_height: number;
  readonly pane_left: number;
  readonly pane_top: number;
  readonly dpr: number;
  readonly hpr: number;
  readonly vpr: number;
  readonly font_size: number;
  readonly font_family: string;
}

/**
 * A pane primitive (reference `IPanePrimitive`). Plain objects and class instances both work — the
 * package binds the methods it finds at attach time.
 */
export interface pane_primitive {
  /** Called once after attach with the owning pane's index. */
  attached?(params: { pane_index: number }): void;
  /** Called once after detach. */
  detached?(): void;
  /** Recompute cached geometry; called each frame before the views are read. */
  update_all_views?(): void;
  /** The views to paint this frame. */
  pane_views?(): primitive_pane_view[];
  /** Boxed labels on the pane's right price scale. */
  price_axis_views?(): primitive_axis_label[];
  /** Boxed labels on the time strip. */
  time_axis_views?(): primitive_axis_label[];
  /**
   * In-pane text draws (Phase 3.5; see {@link primitive_text_view}), painted in
   * the shared pane frame and clipped to the owning pane. Called after `pane_views`
   * in the same render pass, so geometry cached by the view renderers is fresh.
   */
  text_views?(info: primitive_text_context): primitive_text_view[];
  /**
   * Hit test called on hover (reference `IPanePrimitiveBase.hitTest`), in the same absolute
   * bitmap-px coordinate space the draw context uses — cache geometry from the view
   * renderers and test against it here. A hit flows to
   * `mouse_event_params.hovered_object_id` (crosshair-move/click/dbl-click) and applies
   * `cursor_style` to the chart; return `null` for a miss.
   */
  hit_test?(x: number, y: number): primitive_hit_result | null;
}

/** Handle returned by `pane_api.attach_primitive`; drops the primitive from the chart. */
export interface pane_primitive_handle {
  /** Remove the primitive and repaint. Idempotent-ish: detaching twice is a no-op. */
  detach(): void;
}

// ---------------------------------------------------------------------------------------------
// Series primitives (plugin platform Phase C-b) — reference `ISeriesPrimitive` (model/iseries-primitive.ts,
// api/series-api.ts `SeriesApi.attachPrimitive`). Same command-recording model as pane primitives,
// with the binding resolved through the owning series: the views' price converter and the
// price-axis labels use the series' own price scale (its pane's left/right scale as bound, or
// the overlay scale for an overlay series — whose labels land on the right strip, matching the
// engine's last-value rule), and removing the series auto-detaches its primitives.
// ---------------------------------------------------------------------------------------------

/**
 * Draw context handed to a {@link series_primitive_pane_view} renderer. Identical to
 * {@link primitive_draw_context} except `price_to_y` is bound to the OWNING series' price
 * scale (no target argument; percentage/indexed modes anchor on the series' own first visible
 * value, like its geometry).
 */
export interface series_primitive_draw_context extends Omit<primitive_draw_context, "price_to_y"> {
  /** Bitmap y for a price on the owning series' scale; `null` when the scale has no range. */
  price_to_y(price: number): number | null;
}

/** One paintable view of a series primitive (reference `ISeriesPrimitivePaneView`). */
export interface series_primitive_pane_view {
  /** Layer to paint into (default `"normal"`): `bottom` behind the series, `top` above all. */
  z_order?: primitive_z_order;
  /**
   * Record this view's drawing for the current frame. Synchronous; do not stash `ctx`.
   * Runs while the chart is mid-render: do not call chart/series/scale APIs from inside it
   * (the context converters are the read path); capture any other inputs from outside.
   */
  renderer(ctx: series_primitive_draw_context): void;
}

/**
 * A series primitive's axis label descriptor: {@link primitive_axis_label} plus an optional
 * `price`. A finite `price` takes precedence over `coordinate` and is converted on the owning
 * series' scale by the host — a plugin cannot call chart APIs from a mid-render hook, and the
 * draw context's bitmap-px converters don't yield the media-px coordinate the axis strip wants.
 */
export interface series_axis_label extends primitive_axis_label {
  /** Price to place the label at, on the owning series' scale. Wins over `coordinate`. */
  price?: number;
}

/**
 * A series primitive (reference `ISeriesPrimitive`). Plain objects and class instances both work —
 * the package binds the methods it finds at attach time. A hidden series paints no views and
 * contributes no autoscale range (reference gates a series' views and autoscale on its visibility);
 * a pane-less series draws nowhere until re-assigned.
 */
export interface series_primitive {
  /**
   * Called once after attach. `request_update` (injected by the package) schedules a chart
   * repaint — call it after mutating the state the views read (reference `requestUpdate`).
   * `pane_index` is the series' current pane (absent while pane-less).
   */
  attached?(params: { series_id: number; pane_index?: number; request_update?: () => void }): void;
  /** Called once after detach — including the auto-detach when the owning series is removed. */
  detached?(): void;
  /** Recompute cached geometry; called each frame before the views are read. */
  update_all_views?(): void;
  /** The views to paint this frame, in the owning series' pane. */
  pane_views?(): series_primitive_pane_view[];
  /** Boxed labels on the owning series' price scale (overlay series: the right strip). */
  price_axis_views?(): series_axis_label[];
  /** Boxed labels on the time strip. */
  time_axis_views?(): primitive_axis_label[];
  /**
   * In-pane text draws (Phase 3.5; see {@link primitive_text_view}), painted in
   * the shared pane frame and clipped to the owning pane. Called after `pane_views`
   * in the same render pass, so geometry cached by the view renderers is fresh. A hidden series paints no
   * text (its views and autoscale are gated the same way).
   */
  text_views?(info: primitive_text_context): primitive_text_view[];
  /**
   * Autoscale contribution (reference `ISeriesPrimitiveBase.autoscaleInfo`): `{min, max}` price
   * bounds merged into the owning series' price-scale range for the visible logical range
   * `from..to` — use it to expand the scale around graphics drawn outside the data's range.
   * Called every frame while attached; keep it cheap. `null` = no contribution.
   */
  autoscale_info?(from: number, to: number): { min: number; max: number } | null;
  /**
   * Hit test called on hover (reference `ISeriesPrimitiveBase.hitTest`), in the same absolute
   * bitmap-px coordinate space the draw context uses — cache geometry from the view
   * renderers and test against it here. A hit reports the owning series as
   * `mouse_event_params.hovered_series` alongside `hovered_object_id` (the reference's hit source IS
   * the series) and applies `cursor_style` to the chart; return `null` for a miss.
   */
  hit_test?(x: number, y: number): primitive_hit_result | null;
}

/** Handle returned by `series_api.attach_primitive`; drops the primitive from the series. */
export interface series_primitive_handle {
  /** Remove the primitive and repaint. Idempotent-ish: detaching twice is a no-op. */
  detach(): void;
}
