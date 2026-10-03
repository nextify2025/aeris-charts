/**
 * Pointer/wheel/keyboard gesture recognizer wired onto the axis/input overlay canvas.
 *
 * Browser events are normalized here, while pointer membership, gesture transitions, pinch
 * centroid/distance, and primary-touch continuation live in `aeris_charts_engine::interaction` through the WASM
 * input methods. Scale, drawing, trading, kinetic, and animation math remains engine-owned.
 * - touch: cancellable Touch Events, long-press inspection, fixed-centroid zoom-only pinch,
 * - mouse/pen: Pointer Events + pointer capture,
 * - keyboard: arrows pan, +/- zoom, Home fit-content, Escape clear crosshair.
 * All behavior is gated by the resolved gesture config (`chart.gesture_config()`).
 */

import type { chart_impl } from "./impl.js";

const SLOP_MANHATTAN = 5; // px before a press becomes a drag (reference CancelClick/CancelTapManhattanDistance)
const SEP_HIT = 4; // css px hit tolerance around a pane boundary
const LONGPRESS_MS = 240; // touch hold before entering crosshair tracking (reference Delay.LongTap)
const TRADING_TOOLTIP_MS = 450; // hover dwell before a trading control reveals its action tooltip
const TAP_RESET_MS = 500; // window for a second tap to count as a double-tap (reference Delay.ResetClick)
const DBL_TAP_MANHATTAN = 30; // max distance between the taps of a double-tap (reference DoubleTapManhattanDistance)
const DBL_CLICK_MANHATTAN = 5; // max distance between mouse clicks (reference CancelClickManhattanDistance)
const INPUT_UPDATE_LEN = 12;

const enum InputDeviceCode { Mouse = 0, Touch = 1, Pen = 2 }
const enum InputTargetCode { Pane = 0, Drawing = 1, Trading = 2, PriceAxis = 3, TimeAxis = 4, Separator = 5, Alert = 6 }
const enum GestureUpdateCode {
  None = 0, Hover = 1, Pressed = 2, DragStarted = 3, DragMoved = 4,
  PinchStarted = 5, PinchMoved = 6, RebasedSinglePointer = 7, Released = 8,
  Cancelled = 9, LongPress = 10, Rejected = 11,
}

interface input_update {
  kind: GestureUpdateCode;
  pointer_id: number;
  target: InputTargetCode;
  device: InputDeviceCode;
  x: number;
  y: number;
  previous_x: number;
  previous_y: number;
  scale_delta: number;
  active_pointers: number;
  prevent_default: boolean;
}

/** Active axis drag-to-scale session; the engine owns the start snapshot and the formulas. */
type AxisDrag = { kind: "price"; pane: number; target: number } | { kind: "time" };

/** Where a press landed; drives the touch ownership rules (the reference's per-widget handlers). */
type press_region = "pane" | "price_axis" | "time_axis" | "separator";

// ---------------------------------------------------------------------------------------------
// Wheel routing (reference chart-widget.ts `_onMousewheel` + `_determineWheelSpeedAdjustment`),
// shared by the DOM recognizer and the OffscreenCanvas worker façade. Worker-safe: it reads only
// `navigator`, which workers expose too.
// ---------------------------------------------------------------------------------------------

/** The engine surface one routed wheel sample drives. */
export interface wheel_route_target {
  classify_wheel(
    behavior: number,
    delta_x: number,
    delta_y: number,
    delta_mode: number,
    ctrl_key: boolean,
    shift_key: boolean,
  ): number;
  pane_index_at_y(y: number): number;
  price_axis_target_at(pane: number, x: number): number | undefined;
  wheel_zoom_scale(delta_y: number): number;
  price_axis_wheel_zoom(pane: number, target: number, y: number, zoom: number): unknown;
  /** Ordinary wheel zoom of the time scale; the engine owns the anchor. */
  wheel_zoom_time(x: number, scale: number, control: boolean, meta: boolean): void;
  scroll_start(x: number): void;
  scroll_move(x: number): void;
  wheel_scroll_delta(delta: number): number;
  scroll_end(): void;
}

/** One raw wheel sample. `point()` resolves lazily to pane-relative x and chart-relative y. */
export interface wheel_route_sample {
  delta_x: number;
  delta_y: number;
  delta_mode: number;
  ctrl_key: boolean;
  shift_key: boolean;
  /** macOS Cmd; like `ctrl_key`, it zooms around the pointer instead of pinning the right edge. */
  meta_key: boolean;
  /** `_determineWheelSpeedAdjustment` for this sample (see {@link wheel_speed_adjustment}). */
  speed: number;
  point(): { x: number; y: number };
}

export interface wheel_route_config {
  wheel_behavior: "auto" | "pan" | "zoom";
  wheel_zoom: boolean;
  wheel_scroll: boolean;
}

// reference `windowsChrome` = isChromiumBased() && isWindows(), resolved lazily for non-browser runs.
let windows_chrome: boolean | null = null;
function is_windows_chromium(): boolean {
  if (windows_chrome === null) {
    const nav = (typeof navigator === "undefined" ? undefined : navigator) as
      | (Navigator & { userAgentData?: { platform?: string; brands?: { brand: string }[] } })
      | undefined;
    const chromium = nav?.userAgentData?.brands?.some((b) => b.brand.includes("Chromium")) === true;
    const windows = nav?.userAgentData?.platform
      ? nav.userAgentData.platform === "Windows"
      : (nav?.userAgent ?? "").toLowerCase().indexOf("win") >= 0;
    windows_chrome = chromium && windows;
  }
  return windows_chrome;
}

/** reference `_determineWheelSpeedAdjustment`: page/line delta modes use fixed factors, and
 *  Chromium on Windows mis-scales pixel deltas on high-density displays (Chromium issues
 *  1001735 / 1207308), corrected by 1/devicePixelRatio. */
export function wheel_speed_adjustment(delta_mode: number, device_pixel_ratio: number): number {
  switch (delta_mode) {
    case 2: // DOM_DELTA_PAGE: one screen at a time
      return 120;
    case 1: // DOM_DELTA_LINE: one line at a time
      return 32;
  }
  return is_windows_chromium() && device_pixel_ratio > 0 ? 1 / device_pixel_ratio : 1;
}

/** Apply one wheel sample to the time scale (or, in explicit zoom mode, a price axis). Returns
 *  false when the sample does nothing so a DOM host can leave the page scroll alone. */
export function route_wheel(
  wasm: wheel_route_target,
  sample: wheel_route_sample,
  cfg: wheel_route_config,
): boolean {
  const delta_x = (sample.speed * sample.delta_x) / 100;
  const delta_y = -(sample.speed * sample.delta_y) / 100;
  const behavior = cfg.wheel_behavior === "pan" ? 1 : cfg.wheel_behavior === "zoom" ? 2 : 0;
  const intent = wasm.classify_wheel(
    behavior,
    delta_x,
    delta_y,
    sample.delta_mode,
    sample.ctrl_key,
    sample.shift_key,
  );
  const pan_delta = cfg.wheel_behavior === "auto"
    ? delta_x
    : Math.abs(delta_x) >= Math.abs(delta_y) ? delta_x : -delta_y;
  const do_zoom = (intent & 2) !== 0 && delta_y !== 0 && cfg.wheel_zoom;
  const do_scroll = (intent & 1) !== 0 && pan_delta !== 0 && cfg.wheel_scroll;
  if (!do_zoom && !do_scroll) return false;
  if (do_zoom) {
    const point = sample.point();
    const pane = wasm.pane_index_at_y(point.y);
    const target = wasm.price_axis_target_at(pane, point.x) ?? null;
    const zoom = wasm.wheel_zoom_scale(delta_y);
    if (cfg.wheel_behavior === "zoom" && target !== null) {
      // Explicit Aeris `zoom` mode retains price-axis wheel zoom as an extension.
      wasm.price_axis_wheel_zoom(pane, target, point.y, zoom);
    } else {
      // Every chart surface zooms time. The engine owns the anchor: Ctrl/Cmd zooms around the
      // pointer, otherwise the right edge stays pinned (measured TradingView behavior).
      wasm.wheel_zoom_time(point.x, zoom, sample.ctrl_key, sample.meta_key);
    }
  }
  if (do_scroll) {
    // reference `scrollChart(deltaX * -80)`: "80 is a made up coefficient, and minus is for the
    // 'natural' scroll" (engine) — expressed as a scroll session spanning a single jump.
    wasm.scroll_start(0);
    wasm.scroll_move(wasm.wheel_scroll_delta(pan_delta));
    wasm.scroll_end();
  }
  return true;
}

export function install_gestures(chart: chart_impl): () => void {
  const overlay = chart.overlay_el();
  const wasm = chart.wasm;
  // Active captured pointers. The engine resolver owns the canonical membership/state.
  const pointers = new Map<number, { x: number; y: number }>();
  const pointer_targets = new Map<number, InputTargetCode>();
  const input_scratch = new Float64Array(INPUT_UPDATE_LEN);
  const one_touch_x = new Float64Array(1);
  const two_touch_xs = new Float64Array(2);
  let dragging = false; // a time-scale scroll session is active (mouse or touch)
  let sep_drag: { index: number; last_y: number } | null = null;
  let sep_hover = -1; // separator index last reported via set_separator_hover (-1 = none)
  let axis_drag: AxisDrag | null = null;
  let press_start: { x: number; y: number } | null = null;
  let moved = false; // mouse press moved past the click slop (reference _cancelClick)
  // Engine-owned drawing drag (anchor re-anchor or body move) started by a pane press on a
  // drawing (drawings.rs); mutually exclusive with a pan `dragging` session.
  let drawing_dragging = false;
  // Delta Tooltip may intentionally own a mouse drag (brushable Area captures primary-drag).
  // Ordinary primary-drag remains the normal pan path when no tooltip accepts the press.
  let delta_tooltip_dragging = false;
  // Trading controls own the pointer before drawings and chart pan. Moves update only the
  // engine's local preview; confirmed broker state is never mutated by this gesture path.
  let trading_dragging = false;
  let trading_press = false;
  let alert_press = false;
  // The last single click or tap belonged to a trading object or the alert widget. Its pair's
  // second click then acts on no drawing: the pair started on that control, not on a selection.
  let control_click = false;
  // A drawing placement that committed directly on pointer-down. The trailing click/tap is
  // swallowed generically; which placement classes commit on press is engine-owned.
  let creation_press_committed = false;
  // A press consumed by the engine's transient Shift-click measure (start, freeze, or dismiss).
  // Its release and trailing click belong to the measure, never to selection or pane clicks.
  let measure_press = false;
  // Vertical price pan session (reference `startScrollPrice`): the engine holds the range
  // snapshot and shift math; armed only while the scale is NOT in autoscale (its no-op gate).
  let price_pan: { pane: number; target: number } | null = null;

  // Touch-only host state. Gesture classification itself is returned by the engine resolver.
  let active_touch_id: number | null = null;
  let touch_region: press_region = "pane";
  let touch_moved = false;
  let touch_scrolling = false;
  let longpress_timer: ReturnType<typeof setTimeout> | null = null;
  let long_tap_active = false;
  let tap_count = 0;
  let tap_timer: ReturnType<typeof setTimeout> | null = null;
  let tap_position = { x: 0, y: 0 }; // client coords of the first tap
  let mouse_click_timer: ReturnType<typeof setTimeout> | null = null;
  let mouse_click_position = { x: 0, y: 0 };
  let suppress_compatibility_click = false;

  // Touch Events can arbitrate chart manipulation against native page scrolling at runtime.
  let touch_direction: "pending" | "chart" | "page" = "pending";
  let touch_origin = { x: 0, y: 0 };

  // Crosshair tracking mode (reference _startTrackPoint !== null).
  let touch_tracking = false;
  let track_point: { x: number; y: number } | null = null;
  let init_crosshair: { x: number; y: number } | null = null;
  let exit_tracking_on_next_try = false; // reference _exitTrackingModeOnNextTry
  let last_crosshair: { x: number; y: number } | null = null;

  let kinetic_raf: number | null = null; // RAF id while the engine's coast is being driven

  const local_xy = (e: { clientX: number; clientY: number }) => {
    const r = overlay.getBoundingClientRect();
    return { x: e.clientX - r.left - wasm.pane_left(), y: e.clientY - r.top };
  };

  const device_code = (pointer_type: string): InputDeviceCode =>
    pointer_type === "touch" ? InputDeviceCode.Touch
      : pointer_type === "pen" ? InputDeviceCode.Pen : InputDeviceCode.Mouse;
  const region_target = (region: press_region): InputTargetCode => {
    switch (region) {
      case "price_axis": return InputTargetCode.PriceAxis;
      case "time_axis": return InputTargetCode.TimeAxis;
      case "separator": return InputTargetCode.Separator;
      default: return InputTargetCode.Pane;
    }
  };
  /** Axis and separator presses never pan, even while their own drag is switched off. */
  const chrome_target = (target: InputTargetCode | undefined): boolean =>
    target === InputTargetCode.PriceAxis || target === InputTargetCode.TimeAxis
      || target === InputTargetCode.Separator;
  const read_input_update = (): input_update => ({
    kind: input_scratch[0] as GestureUpdateCode,
    pointer_id: input_scratch[2]!,
    target: input_scratch[3] as InputTargetCode,
    device: input_scratch[4] as InputDeviceCode,
    x: input_scratch[5]!,
    y: input_scratch[6]!,
    previous_x: input_scratch[7]!,
    previous_y: input_scratch[8]!,
    scale_delta: input_scratch[9]!,
    active_pointers: input_scratch[10]!,
    prevent_default: input_scratch[11] !== 0,
  });
  const feed_pointer = (
    phase: "down" | "move" | "up",
    event: PointerEvent,
    target = pointer_targets.get(event.pointerId) ?? InputTargetCode.Pane,
  ): input_update => {
    const point = local_xy(event);
    const method = phase === "down" ? wasm.input_pointer_down.bind(wasm)
      : phase === "move" ? wasm.input_pointer_move.bind(wasm) : wasm.input_pointer_up.bind(wasm);
    const modifiers = (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0)
      | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
    method(
      event.pointerId,
      device_code(event.pointerType),
      target,
      modifiers,
      point.x,
      point.y,
      event.timeStamp || performance.now(),
      event.pressure,
      event.tiltX,
      event.tiltY,
      input_scratch,
    );
    return read_input_update();
  };
  const set_touch_action = () => {
    // Static `touch-action` cannot express the reference's direction-dependent arbitration.
    // Cancellable Touch Events below decide after the 5 px slop instead.
    overlay.style.touchAction = "auto";
  };

  const separator_at = (y: number, touch = false): number => {
    const tolerance = touch ? 12 : SEP_HIT;
    const ys = wasm.pane_separator_ys();
    for (let i = 0; i < ys.length; i++) {
      if (Math.abs(y - ys[i]!) <= tolerance) return i;
    }
    return -1;
  };

  /** Report the hovered separator to the engine (-1 = none); repaint only when it changes. */
  const set_sep_hover = (index: number) => {
    if (index === sep_hover) return;
    sep_hover = index;
    wasm.set_separator_hover(index);
    chart.repaint();
  };

  /** Exact price-axis target under `p`, including pane-local named scales. */
  const price_axis_target_at = (p: { x: number; y: number }): number | null => {
    return wasm.price_axis_target_at(pane_of(p.y), p.x) ?? null;
  };
  const is_time_axis = (p: { x: number; y: number }): boolean =>
    p.y > overlay.getBoundingClientRect().height - wasm.time_scale_height();
  const region_of = (p: { x: number; y: number }, touch = false): press_region => {
    if (separator_at(p.y, touch) >= 0) return "separator";
    if (price_axis_target_at(p) !== null) return "price_axis";
    if (is_time_axis(p)) return "time_axis";
    return "pane";
  };
  const pane_of = (y: number): number => wasm.pane_index_at_y(y);

  // A trading control's action tooltip waits out a hover dwell instead of appearing on contact,
  // so sweeping the pointer across stacked markers never flashes one tooltip per marker. The
  // engine is headless and owns no timer, so the dwell lives here and arms the engine when it
  // elapses.
  let tooltip_timer: ReturnType<typeof setTimeout> | null = null;
  const clear_tooltip_dwell = () => {
    if (tooltip_timer === null) return;
    clearTimeout(tooltip_timer);
    tooltip_timer = null;
  };
  const start_tooltip_dwell = (x: number, y: number) => {
    clear_tooltip_dwell();
    if (chart.trading_hit_at(x, y)?.kind !== "cancel_button") return;
    tooltip_timer = setTimeout(() => {
      tooltip_timer = null;
      if (chart.arm_trading_tooltip()) chart.repaint();
    }, TRADING_TOOLTIP_MS);
  };

  const set_crosshair = (x: number, y: number) => {
    last_crosshair = { x, y };
    // The engine keeps this position for callbacks and snapping while suppressing its visual
    // crosshair whenever an interactive trading object or drawing owns pointer feedback.
    wasm.set_crosshair(x, y);
    // Phase C-d: refresh the hover hit-test (primitives + series) before the repaint that
    // follows, so a hovered series' `hoveredSeriesOnTop` z-bump lands on the same frame.
    chart.update_hover(x, y);
    // Only a changed hover restarts the dwell — holding still over one control lets it elapse.
    if (chart.trading_hover_at(x, y)) start_tooltip_dwell(x, y);
    chart.emit_crosshair(x, y);
  };

  // the public reference's Ctrl-held magnet, scoped to DRAWING work: the Normal-mode crosshair snaps
  // to the hovered bar's rendered prices only while a drawing tool is armed (anchor
  // placement/preview) and the engine's effective drawing magnet is strong — the chart/tool
  // magnet mode with Ctrl/Cmd as its temporary toggle — or an existing drawing is being dragged
  // with Ctrl/Cmd held. Plain browsing never price-snaps on Ctrl. Forwarded on every pointer
  // move/down and on modifier key events, so a press/release without mouse movement still
  // refreshes the snap live.
  const apply_crosshair_magnet = (e: { ctrlKey: boolean; metaKey: boolean }) => {
    const toggle = e.ctrlKey || e.metaKey;
    wasm.set_crosshair_ohlc_magnet(
      (chart.creation_armed() && chart.armed_drawing_magnet_strong(toggle)) || (drawing_dragging && toggle),
    );
  };
  const on_modifier_key = (e: KeyboardEvent) => {
    if (e.key !== "Control" && e.key !== "Meta") return;
    apply_crosshair_magnet(e);
    if (last_crosshair !== null) {
      set_crosshair(last_crosshair.x, last_crosshair.y);
      chart.repaint();
    }
  };

  const clear_longpress = () => {
    if (longpress_timer !== null) {
      clearTimeout(longpress_timer);
      longpress_timer = null;
    }
  };
  const reset_tap = () => {
    if (tap_timer !== null) {
      clearTimeout(tap_timer);
      tap_timer = null;
    }
    tap_count = 0;
  };
  const reset_mouse_click = () => {
    if (mouse_click_timer !== null) {
      clearTimeout(mouse_click_timer);
      mouse_click_timer = null;
    }
  };
  const arm_mouse_click = (e: MouseEvent) => {
    mouse_click_position = { x: e.clientX, y: e.clientY };
    mouse_click_timer = setTimeout(reset_mouse_click, TAP_RESET_MS);
  };

  const stop_kinetic = () => {
    wasm.kinetic_stop();
    if (kinetic_raf !== null) {
      cancelAnimationFrame(kinetic_raf);
      kinetic_raf = null;
    }
  };
  /** Drive the engine's kinetic coast. The drag session closes before the coast starts, matching
   *  the reference's rightOffset animation handoff; the host only schedules frames. */
  const start_kinetic = () => {
    const now = performance.now();
    const engaged = wasm.kinetic_release(wasm.scroll_position(), now);
    wasm.scroll_end();
    if (!engaged) {
      return;
    }
    const step = () => {
      const t = performance.now();
      const position = wasm.kinetic_position(t);
      if (!Number.isNaN(position)) wasm.scroll_to_position(position);
      chart.repaint();
      if (wasm.kinetic_finished(t)) {
        wasm.kinetic_stop();
        kinetic_raf = null;
        return;
      }
      kinetic_raf = requestAnimationFrame(step);
    };
    kinetic_raf = requestAnimationFrame(step);
  };

  /** Open a scroll session and (when the device wants a coast) start engine-side sampling. */
  const begin_scroll = (x: number, kind: "mouse" | "touch") => {
    wasm.scroll_start(x);
    dragging = true;
    const cfg = chart.gesture_config();
    const enabled = kind === "touch" ? cfg.kinetic_touch : cfg.kinetic_mouse;
    wasm.kinetic_begin_sampling(enabled, wasm.scroll_position(), performance.now());
  };
  /** Resolve the exact scale owned by the selected/hit series without opening a drag snapshot. */
  const resolve_price_pan = (pane: number, x: number, y: number) => {
    disarm_price_pan();
    const target = wasm.price_pan_target_at(pane, x, y) ?? null;
    if (target === null) return;
    price_pan = { pane, target };
  };
  /** Open the already-resolved price scale at the resolver's threshold-crossing sample. */
  const start_price_pan = (y: number) => {
    if (price_pan === null) return;
    if (wasm.price_scale_auto_scale(price_pan.pane, price_pan.target) !== false) {
      price_pan = null;
      return;
    }
    wasm.price_axis_start_scroll(price_pan.pane, price_pan.target, y);
  };
  /** Close the engine's price-pan session (a no-op arm leaves nothing to close). */
  const disarm_price_pan = () => {
    if (price_pan === null) return;
    wasm.price_axis_end_scroll(price_pan.pane, price_pan.target);
    price_pan = null;
  };
  /** End a pan drag: coast when the flick qualifies, otherwise just close the session. */
  const end_drag = (kind: "mouse" | "touch") => {
    if (!dragging) {
      disarm_price_pan();
      return;
    }
    dragging = false;
    touch_scrolling = false;
    trading_press = false;
    alert_press = false;
    disarm_price_pan();
    const cfg = chart.gesture_config();
    const enabled =
      (kind === "touch" ? cfg.kinetic_touch : cfg.kinetic_mouse) && !chart.prefers_reduced_motion();
    if (enabled) {
      start_kinetic();
    } else {
      wasm.kinetic_stop();
      wasm.scroll_end();
    }
  };

  const apply_axis_drag = (p: { x: number; y: number }) => {
    if (axis_drag === null) return;
    // The engine owns the start snapshot and the scale formulas (reference `PriceScale.scaleTo`
    // / `TimeScale.scaleTo`); the recognizer just forwards the drag position.
    if (axis_drag.kind === "price") {
      wasm.price_axis_scale_to(axis_drag.pane, axis_drag.target, p.y);
    } else {
      wasm.time_axis_scale_to(p.x);
    }
  };
  const apply_sep_drag = (p: { x: number; y: number }) => {
    if (sep_drag === null) return;
    const dy = p.y - sep_drag.last_y;
    sep_drag.last_y = p.y;
    wasm.drag_pane_separator(sep_drag.index, dy);
  };
  const apply_price_pan = (y: number) => {
    if (price_pan === null) return;
    // Vertical price pan (reference `scrollPriceTo`): the engine shifts its armed snapshot by
    // dy * span/(h-1) — drag down moves the range up, so the candles follow the cursor.
    wasm.price_axis_scroll_to(price_pan.pane, price_pan.target, y);
  };

  /** Arm the press-region interaction shared by mousedown and touchstart; returns the region. */
  const arm_press = (p: { x: number; y: number }, touch = false): press_region => {
    const cfg = chart.gesture_config();
    sep_drag = null;
    end_axis_drag();
    disarm_price_pan();
    // separator drag takes precedence over any pan/scale (reference layout.panes.enableResize gates it)
    const si = separator_at(p.y, touch);
    if (si >= 0) {
      if (cfg.panes_resize) {
        sep_drag = { index: si, last_y: p.y };
        // The drag itself highlights the separator; clear the hover highlight.
        set_sep_hover(-1);
      }
      return "separator";
    }
    // axis drag-to-scale: price axis (vertical) / time axis (horizontal)
    const price_target = price_axis_target_at(p);
    if (price_target !== null) {
      const pane = pane_of(p.y);
      // reference `PriceScale.scaleTo` is a no-op in percentage and indexed-to-100 modes (and on
      // an empty scale) — the engine reports whether a drag can scale at all.
      if (cfg.axis_scale_price && wasm.price_axis_scalable(pane, price_target)) {
        axis_drag = { kind: "price", pane, target: price_target };
        wasm.price_axis_start_scale(pane, price_target, p.y);
      }
      return "price_axis"; // never pan from an axis strip
    }
    if (is_time_axis(p)) {
      if (cfg.axis_scale_time) {
        axis_drag = { kind: "time" };
        wasm.time_axis_start_scale(p.x);
      }
      return "time_axis";
    }
    return "pane";
  };
  /** Close the engine's axis scale session (reference `endScale` on pointer release). */
  const end_axis_drag = () => {
    if (axis_drag === null) return;
    if (axis_drag.kind === "price") {
      wasm.price_axis_end_scale(axis_drag.pane, axis_drag.target);
    } else {
      wasm.time_axis_end_scale();
    }
    axis_drag = null;
  };

  // ---------------------------------------------------------------------------------------------
  // Wheel: the shared `route_wheel` owns the semantics; this host adds event-default policy.
  // ---------------------------------------------------------------------------------------------

  const on_wheel = (e: WheelEvent) => {
    const handled = route_wheel(
      wasm,
      {
        delta_x: e.deltaX,
        delta_y: e.deltaY,
        delta_mode: e.deltaMode,
        ctrl_key: e.ctrlKey,
        shift_key: e.shiftKey,
        meta_key: e.metaKey,
        speed: wheel_speed_adjustment(e.deltaMode, window.devicePixelRatio),
        point: () => local_xy(e),
      },
      chart.gesture_config(),
    );
    if (!handled) return; // let the page scroll
    if (e.cancelable) e.preventDefault();
    chart.repaint();
  };

  // ---------------------------------------------------------------------------------------------
  // Mouse / pen (the same engine resolver also receives the touch path below)
  // ---------------------------------------------------------------------------------------------

  const on_down = (e: PointerEvent) => {
    if (e.pointerType === "touch") return;
    if (e.button !== 0) return; // primary button only (reference _mouseDownHandler)
    // Any mouse activity cancels touch tracking mode (reference `_onMouseEvent`).
    touch_tracking = false;
    track_point = null;
    stop_kinetic();
    stop_scroll_anim();
    try {
      overlay.setPointerCapture(e.pointerId);
    } catch {
      // ignore synthetic events with no active pointer
    }
    const p = local_xy(e);
    pointers.set(e.pointerId, p);
    apply_crosshair_magnet(e);
    // Any active pointer pauses the countdown timer (no mid-gesture repaint/lag).
    if (pointers.size === 1) chart.set_interacting(true);
    if (pointers.size !== 1) return;
    press_start = p;
    moved = false;
    measure_press = false;
    const region = arm_press(p);
    if (region !== "pane") {
      const target = region_target(region);
      pointer_targets.set(e.pointerId, target);
      feed_pointer("down", e, target);
      return;
    }
    const magnet = e.ctrlKey || e.metaKey;
    // A live measure owns the next pane press: it freezes a following measure or dismisses a
    // frozen one before any object under the pointer is considered.
    const claim_measure = () => {
      measure_press = true;
      pointer_targets.set(e.pointerId, InputTargetCode.Drawing);
      feed_pointer("down", e, InputTargetCode.Drawing);
      set_crosshair(p.x, p.y);
      chart.repaint();
    };
    if (wasm.measure_active() && wasm.measure_pointer_down(p.x, p.y, false, magnet)) {
      claim_measure();
      return;
    }
    const trading_hit = chart.trading_hit_at(p.x, p.y);
    if (trading_hit !== null) {
      pointer_targets.set(e.pointerId, InputTargetCode.Trading);
      feed_pointer("down", e, InputTargetCode.Trading);
      chart.trading_pressed_at(p.x, p.y);
      trading_dragging = chart.trading_drag_start_at(p.x, p.y);
      set_crosshair(p.x, p.y);
      chart.repaint();
      return;
    }
    if (chart.alert_create_hit_at(p.x, p.y)) {
      pointer_targets.set(e.pointerId, InputTargetCode.Alert);
      feed_pointer("down", e, InputTargetCode.Alert);
      alert_press = true;
      set_crosshair(p.x, p.y);
      chart.repaint();
      return;
    }
    chart.deactivate_trading_group();
    // Armed drawing tools own pane presses. The engine controller decides whether this placement
    // class starts capture, commits on press, or waits for the click activation.
    if (chart.creation_armed()) {
      pointer_targets.set(e.pointerId, InputTargetCode.Drawing);
      feed_pointer("down", e, InputTargetCode.Drawing);
      if (chart.creation_pointer_down(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey)) {
        creation_press_committed = true;
        // A press-committed placement may open a host editor. Keep the overlay's native focus
        // grab from immediately blurring that editor.
        e.preventDefault();
      }
      set_crosshair(p.x, p.y);
      chart.repaint();
      return;
    }
    // Snapshot the selection before the drag grab selects the hit — `emit_click`'s
    // two-step text editing needs the pre-press selection.
    chart.note_drawing_press();
    if (wasm.drawing_drag_start_at(p.x, p.y)) {
      pointer_targets.set(e.pointerId, InputTargetCode.Drawing);
      feed_pointer("down", e, InputTargetCode.Drawing);
      drawing_dragging = true;
      set_crosshair(p.x, p.y);
      chart.repaint();
      return;
    }
    // A Delta Tooltip gets first refusal on the pane gesture. Brushable Area intentionally uses
    // that capture so primary dragging compares instead of starting a competing canvas pan.
    delta_tooltip_dragging = chart.native_delta_tooltip_mouse_down(p.x, e.shiftKey);
    // Shift on empty chart space starts the engine's transient measure: pulling up measures a
    // rise, pulling down a fall, in any direction.
    if (!delta_tooltip_dragging && e.shiftKey && wasm.measure_pointer_down(p.x, p.y, true, magnet)) {
      claim_measure();
      return;
    }
    if (!delta_tooltip_dragging && chart.gesture_config().pan) {
      // Resolve the directly hit/selected scale at press time while the pointer is still on its
      // geometry. This snapshots only; no scale mutates before the resolver opens the drag.
      resolve_price_pan(pane_of(p.y), p.x, p.y);
    }
    pointer_targets.set(e.pointerId, InputTargetCode.Pane);
    feed_pointer("down", e, InputTargetCode.Pane);
    // reference `mouseDownEvent` places the crosshair at the press point.
    set_crosshair(p.x, p.y);
    chart.repaint();
  };

  const on_move = (e: PointerEvent) => {
    if (e.pointerType === "touch") return;
    // Any mouse activity cancels touch tracking mode (reference `_onMouseEvent`).
    touch_tracking = false;
    track_point = null;
    // Ignore moves driven by a non-primary button drag (reference `_mouseMoveWithDownHandler`); a
    // hover (buttons === 0) or a left-drag (bit 0 set) passes.
    if (e.buttons !== 0 && (e.buttons & 1) === 0) return;
    const p = local_xy(e);
    const update = feed_pointer("move", e);
    chart.native_delta_tooltip_mouse_move(p.x);
    apply_crosshair_magnet(e);

    if (pointers.has(e.pointerId)) pointers.set(e.pointerId, p);
    if (pointers.size > 0 && press_start !== null && !moved) {
      // reference CancelClickManhattanDistance = 5 (Manhattan).
      moved = Math.abs(p.x - press_start.x) + Math.abs(p.y - press_start.y) >= SLOP_MANHATTAN;
      if (moved) reset_mouse_click();
    }

    // Axis and separator sessions may snapshot on press, but cannot mutate until the resolver
    // reaches the shared 5 px threshold. The crossing sample applies their upstream formula.
    if (axis_drag !== null) {
      if (update.kind === GestureUpdateCode.DragStarted || update.kind === GestureUpdateCode.DragMoved) {
        apply_axis_drag(p);
      }
      chart.repaint();
      return;
    }
    if (sep_drag !== null) {
      if (update.kind === GestureUpdateCode.DragStarted || update.kind === GestureUpdateCode.DragMoved) {
        apply_sep_drag(p);
      }
      // The separator is chrome (reference pane-separator.ts): the crosshair hides during the
      // resize drag instead of freezing mid-pane at the grab point.
      if (last_crosshair !== null) {
        last_crosshair = null;
        wasm.clear_crosshair();
        chart.clear_hover();
        chart.emit_crosshair_left();
      }
      chart.repaint();
      return;
    }

    // Separator hover highlight (no button pressed). The cursor itself is resolved after
    // the crosshair feed below, which refreshes the hover hit-test first.
    if (pointers.size === 0) {
      // Same gate as the row-resize cursor below: no hover highlight while resizing is off.
      set_sep_hover(chart.gesture_config().panes_resize ? separator_at(p.y) : -1);
    }

    if (wasm.measure_active()) {
      // Follows with or without a held button: press-drag-release and click-move-click both
      // measure.
      wasm.measure_pointer_move(p.x, p.y, e.ctrlKey || e.metaKey);
    } else if (trading_dragging) {
      chart.trading_drag_to(p.y);
    } else if (chart.creation_armed()) {
      chart.creation_pointer_move(
        p.x,
        p.y,
        e.ctrlKey || e.metaKey,
        e.shiftKey,
        (e.buttons & 1) !== 0,
      );
    } else if (drawing_dragging) {
      // Engine-owned anchor/body drag (drawings.rs): the engine re-anchors from the start
      // snapshot; the crosshair feed below keeps tracking the cursor. Modifier keys are
      // forwarded live (toggling mid-drag responds immediately, reference-informed behavior): Ctrl/Cmd =
      // magnet (snap anchors to the nearest rendered bar price), Shift = straighten
      // (0°/45°/90° anchor constraint, dominant-axis body move). Ctrl never straightens.
      wasm.drawing_drag_to(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey);
    } else if (delta_tooltip_dragging) {
      // The native comparison interaction explicitly owns this pane drag.
    } else if (
      update.kind === GestureUpdateCode.DragStarted && chart.gesture_config().pan
      && !chrome_target(pointer_targets.get(e.pointerId))
    ) {
      // Pane panning opens on the threshold sample; like the public reference, movement begins on
      // the following sample. Kinetic sampling starts only after this transition.
      begin_scroll(p.x, "mouse");
      start_price_pan(p.y);
    } else if (update.kind === GestureUpdateCode.DragMoved && dragging) {
      wasm.scroll_move(p.x);
      wasm.kinetic_add_sample(wasm.scroll_position(), performance.now());
      apply_price_pan(p.y);
    }
    // Crosshair: a hover over an axis strip is a pane mouseleave in the reference (its axis
    // strips are separate widgets) — the crosshair HIDES, and the hovered-source state clears
    // with it. The pane separator is chrome the same way (reference pane-separator.ts), so a
    // separator hover hides the crosshair too instead of pinning it onto the divider. During an
    // active captured drag keep feeding positions; the engine clamps them into the pane (the
    // reference's document-level drag listeners do the same).
    if (pointers.size > 0 || (price_axis_target_at(p) === null && !is_time_axis(p) && separator_at(p.y) < 0)) {
      set_crosshair(p.x, p.y);
    } else if (last_crosshair !== null) {
      last_crosshair = null;
      wasm.clear_crosshair();
      chart.clear_hover();
      chart.emit_crosshair_left();
    }
    // Hover cursor feedback (no button pressed), resolved AFTER the crosshair feed refreshed
    // the hover hit-test, so a primitive's `hit_test` cursor applies on the same move it
    // starts hitting (reference applies the hovered source's cursorStyle on every crosshair move).
    if (pointers.size === 0) {
      const region_cursor =
        separator_at(p.y) >= 0 && chart.gesture_config().panes_resize
          ? "row-resize"
          : price_axis_target_at(p) !== null
            ? (chart.gesture_config().axis_scale_price &&
                wasm.price_axis_scalable(pane_of(p.y), price_axis_target_at(p)!) ? "ns-resize" : "default")
            : is_time_axis(p)
              ? (chart.gesture_config().axis_scale_time ? "ew-resize" : "default")
              : "crosshair";
      // A primitive's cursor overrides the region cursor while its hit holds — but only over
      // the pane (the hover state is not refreshed over the axis strips). A series hit shows
      // the click affordance (industry-standard: a series is selectable), falling back to the
      // region cursor off the geometry.
      // A live measure keeps the measuring crosshair cursor over every chart object.
      overlay.style.cursor =
        region_cursor === "crosshair" && !wasm.measure_active()
          ? (chart.trading_cursor_at(p.x, p.y) ?? (chart.alert_create_hit_at(p.x, p.y) ? "pointer" : null) ??
            chart.hover_cursor() ?? (chart.hover_series_id() !== null ? "pointer" : region_cursor))
          : region_cursor;
    }
    chart.repaint();
  };

  const end_pointer = (e: PointerEvent) => {
    if (e.pointerType === "touch") return;
    if (e.button !== 0) return; // primary button only (reference _mouseUpHandler)
    // Any mouse activity cancels touch tracking mode (reference `_onMouseEvent`).
    touch_tracking = false;
    track_point = null;
    const p = local_xy(e);
    chart.native_delta_tooltip_mouse_up();
    delta_tooltip_dragging = false;
    feed_pointer("up", e);
    pointers.delete(e.pointerId);
    pointer_targets.delete(e.pointerId);
    chart.clear_trading_pressed();
    if (pointers.size === 0) chart.set_interacting(false);
    if (pointers.size !== 0) return;
    if (sep_drag !== null) {
      sep_drag = null;
      return;
    }
    if (axis_drag !== null) {
      end_axis_drag();
      return;
    }
    if (measure_press) {
      wasm.measure_pointer_up(p.x, p.y, e.ctrlKey || e.metaKey);
      chart.repaint();
      return;
    }
    if (chart.creation_capture_active()) {
      chart.creation_pointer_up(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey);
      chart.repaint();
      return;
    }
    if (chart.creation_armed()) {
      chart.creation_pointer_up(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey);
    }
    if (trading_dragging) {
      trading_dragging = false;
      chart.trading_drag_end();
      chart.repaint();
      return;
    }
    if (drawing_dragging) {
      // End the engine's drawing drag (no coast, no scroll session to close — the pan path
      // never started). The click that follows (no move) routes to selection.
      drawing_dragging = false;
      wasm.drawing_drag_end();
      apply_crosshair_magnet(e);
      chart.repaint();
      return;
    }
    // reference `mouseUpEvent` ends the scroll (maybe starting a kinetic coast) but never hides the
    // crosshair — that only happens on mouse leave, Escape, or a touch end.
    end_drag("mouse");
    chart.repaint();
  };

  const on_cancel = (e: PointerEvent) => {
    if (e.pointerType === "touch") return;
    cancel_active_input();
  };

  const on_leave = (e: PointerEvent) => {
    if (e.pointerType !== "mouse") return;
    // reference `mouseLeaveEvent` hides the crosshair; an active captured drag is left alone.
    if (pointers.size > 0) return;
    chart.native_delta_tooltip_leave();
    set_sep_hover(-1);
    wasm.set_crosshair_ohlc_magnet(false); // release the Ctrl-magnet with the hover
    chart.clear_hover(); // Phase C-d: release the hover hit + hovered-series z-bump
    clear_tooltip_dwell();
    chart.clear_trading_hover();
    wasm.clear_crosshair();
    chart.emit_crosshair_left();
    chart.repaint();
  };

  const run_dblclick = (x: number, y: number) => {
    const cfg = chart.gesture_config();
    const region = region_of({ x, y });
    if (region === "time_axis") {
      // reference time-axis-widget mouseDoubleClickEvent (handleScale.axisDoubleClickReset.time).
      if (cfg.axis_dblclick_reset_time) {
        wasm.reset_time_scale();
        chart.repaint();
      }
      return;
    }
    if (region === "price_axis") {
      // reference price-axis-widget mouseDoubleClickEvent (handleScale.axisDoubleClickReset.price).
      const target = price_axis_target_at({ x, y });
      if (cfg.axis_dblclick_reset_price && target !== null) {
        wasm.reset_price_scale(pane_of(y), target);
        chart.repaint();
      }
      return;
    }
    if (region === "separator") return;
    if (chart.creation_finish()) {
      chart.repaint();
      return;
    }
    chart.emit_dbl_click(x, y);
  };

  const run_single_click = (e: MouseEvent, p: { x: number; y: number }) => {
    const pressed_alert = alert_press;
    alert_press = false;
    control_click = false;
    // Axis widgets own their clicks and never emit pane click callbacks.
    if (region_of(p) !== "pane") return;
    // A placement that committed on pointer-down swallows its trailing compatibility click.
    if (creation_press_committed) {
      creation_press_committed = false;
      return;
    }
    const trading_hit = chart.trading_hit_at(p.x, p.y);
    if (trading_hit !== null) {
      control_click = true;
      chart.trading_activate_at(p.x, p.y);
      chart.repaint();
      return;
    }
    if (pressed_alert && chart.alert_create_hit_at(p.x, p.y)) {
      control_click = true;
      chart.activate_alert_create_at(p.x, p.y);
      chart.repaint();
      return;
    }
    // An armed drawing tool consumes pane clicks for anchor placement (engine-owned creation);
    // modifiers snap the placed anchor (Ctrl = magnet to OHLC, Shift = straighten).
    if (chart.creation_armed() && chart.creation_click(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey)) {
      chart.repaint();
      return;
    }
    chart.emit_click(p.x, p.y);
  };

  const on_click = (e: MouseEvent) => {
    if (measure_press) {
      measure_press = false;
      reset_mouse_click();
      return;
    }
    if (moved) return;
    if (suppress_compatibility_click) {
      suppress_compatibility_click = false;
      return;
    }
    const p = local_xy(e);
    // A newly armed Aeris drawing tool explicitly owns its first placement click. It cannot be
    // paired with a click from the interaction that armed or preceded the tool.
    if (chart.creation_armed() && !chart.creation_sequence_active()) {
      reset_mouse_click();
      run_single_click(e, p);
      if (chart.creation_sequence_active()) arm_mouse_click(e);
      return;
    }
    if (mouse_click_timer === null) {
      arm_mouse_click(e);
      // the public reference emits the first single click immediately.
      run_single_click(e, p);
      return;
    }

    const distance = Math.abs(e.clientX - mouse_click_position.x)
      + Math.abs(e.clientY - mouse_click_position.y);
    reset_mouse_click();
    // A qualifying second click emits only double-click. Variable drawing sequences still need
    // the second terminal anchor before the shared finish action. An already-hit Aeris drawing
    // may consume the click internally (for example, opening its text editor), but pane click
    // subscribers still receive only the first single click.
    if (distance < DBL_CLICK_MANHATTAN) {
      if (chart.creation_sequence_active()) {
        chart.creation_click(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey);
      } else if (region_of(p) === "pane" && !control_click) {
        chart.activate_drawing_double_click(p.x, p.y);
      }
      run_dblclick(p.x, p.y);
      return;
    }
    // A click outside the double-click radius starts a fresh recognition window and is still
    // emitted immediately. This is essential for rapid multi-anchor drawing placement and is
    // the reference handler's ordinary-click path, not a cancelled second click.
    const was_creation = chart.creation_armed();
    arm_mouse_click(e);
    run_single_click(e, p);
    if (was_creation && !chart.creation_armed()) reset_mouse_click();
  };

  const on_contextmenu = (e: MouseEvent) => {
    const p = local_xy(e);
    if (chart.emit_chart_context(p.x, p.y)) e.preventDefault();
  };

  // reference `preventScrollByWheelClick` (helpers/events.ts): suppress Chrome's middle-click
  // autoscroll; registered Chrome-only like reference (`window.chrome !== undefined`).
  const on_mousedown = (e: MouseEvent) => {
    if (e.button === 1) e.preventDefault();
  };
  const is_chrome = (window as unknown as { chrome?: unknown }).chrome !== undefined;

  // ---------------------------------------------------------------------------------------------
  // Touch direct manipulation (Touch Events normalized into the shared Rust resolver)
  // ---------------------------------------------------------------------------------------------

  const finish_scroll_without_coast = () => {
    if (!dragging) return;
    dragging = false;
    touch_scrolling = false;
    disarm_price_pan();
    wasm.kinetic_stop();
    wasm.scroll_end();
  };

  const cancel_active_input = () => {
    wasm.input_cancel_all(input_scratch);
    clear_longpress();
    reset_tap();
    finish_scroll_without_coast();
    end_axis_drag();
    sep_drag = null;
    if (chart.creation_capture_active()) chart.cancel_active_drawing_creation();
    if (trading_dragging) {
      trading_dragging = false;
      chart.cancel_trading_drag();
    }
    chart.clear_trading_pressed();
    if (drawing_dragging) {
      drawing_dragging = false;
      wasm.drawing_drag_cancel();
    }
    measure_press = false;
    wasm.cancel_measure();
    wasm.set_crosshair_ohlc_magnet(false);
    trading_press = false;
    alert_press = false;
    touch_tracking = false;
    track_point = null;
    init_crosshair = null;
    active_touch_id = null;
    pointers.clear();
    pointer_targets.clear();
    chart.set_interacting(false);
    chart.native_delta_tooltip_mouse_up();
    delta_tooltip_dragging = false;
    chart.repaint();
  };

  const on_touch_pointer_down = (e: PointerEvent) => {
    stop_kinetic();
    stop_scroll_anim();
    const p = local_xy(e);
    try { overlay.setPointerCapture(e.pointerId); } catch { /* synthetic pointer */ }
    pointers.set(e.pointerId, p);
    chart.set_interacting(true);

    if (pointers.size > 1) {
      pointer_targets.set(e.pointerId, InputTargetCode.Pane);
      const update = feed_pointer("down", e, InputTargetCode.Pane);
      if (update.kind === GestureUpdateCode.Rejected) {
        pointers.delete(e.pointerId);
        pointer_targets.delete(e.pointerId);
        try { overlay.releasePointerCapture(e.pointerId); } catch { /* already released */ }
        return;
      }
      clear_longpress();
      reset_tap();
      touch_tracking = false;
      track_point = null;
      touch_moved = true;
      trading_press = false;
      alert_press = false;
      if (drawing_dragging) {
        drawing_dragging = false;
        wasm.drawing_drag_cancel();
      }
      if (trading_dragging) {
        trading_dragging = false;
        chart.cancel_trading_drag();
      }
      if (chart.creation_capture_active()) chart.cancel_active_drawing_creation();
      finish_scroll_without_coast();
      end_axis_drag();
      sep_drag = null;
      if (update.kind === GestureUpdateCode.PinchStarted) {
        if (e.cancelable && update.prevent_default) e.preventDefault();
      }
      return;
    }

    active_touch_id = e.pointerId;
    press_start = p;
    touch_region = region_of(p, true);
    touch_moved = false;
    touch_scrolling = false;
    long_tap_active = false;
    exit_tracking_on_next_try = touch_tracking;
    if (touch_tracking && last_crosshair !== null) {
      init_crosshair = last_crosshair;
      track_point = p;
    }

    const region = arm_press(p, true);
    let target = region_target(region);
    if (region === "pane") {
      if (chart.trading_hit_at_device(p.x, p.y, InputDeviceCode.Touch) !== null) {
        target = InputTargetCode.Trading;
        trading_press = true;
        chart.trading_pressed_at_device(p.x, p.y, InputDeviceCode.Touch);
        trading_dragging = chart.trading_drag_start_at_device(p.x, p.y, InputDeviceCode.Touch);
      } else if (chart.alert_create_hit_at(p.x, p.y)) {
        target = InputTargetCode.Alert;
        alert_press = true;
      } else if (chart.creation_armed()) {
        target = InputTargetCode.Drawing;
        creation_press_committed = chart.creation_pointer_down(p.x, p.y, false, false);
      } else {
        // Same pre-grab selection snapshot as the mouse path (two-step text editing).
        chart.note_drawing_press();
        if (wasm.drawing_drag_start_at_device(p.x, p.y, InputDeviceCode.Touch)) {
          target = InputTargetCode.Drawing;
          drawing_dragging = true;
        } else {
          chart.deactivate_trading_group();
        }
      }
    }
    pointer_targets.set(e.pointerId, target);
    const update = feed_pointer("down", e, target);
    if (update.kind === GestureUpdateCode.Rejected) {
      cancel_active_input();
      return;
    }
    if (target === InputTargetCode.Pane) {
      const cfg = chart.gesture_config();
      if (cfg.pan_horz_touch || cfg.pan_vert_touch) resolve_price_pan(pane_of(p.y), p.x, p.y);
    }

    clear_longpress();
    if (target === InputTargetCode.Pane) {
      longpress_timer = setTimeout(() => {
        longpress_timer = null;
        wasm.input_long_press(e.pointerId, input_scratch);
        const longpress = read_input_update();
        if (longpress.kind !== GestureUpdateCode.LongPress || press_start === null) return;
        long_tap_active = true;
        touch_tracking = true;
        exit_tracking_on_next_try = false;
        track_point = press_start;
        init_crosshair = press_start;
        set_crosshair(press_start.x, press_start.y);
        chart.repaint();
      }, LONGPRESS_MS);
    }

    if (tap_timer === null) {
      tap_count = 0;
      control_click = false; // a new tap pair starts here
      tap_timer = setTimeout(reset_tap, TAP_RESET_MS);
      tap_position = { x: e.clientX, y: e.clientY };
    }
  };

  const on_touch_pointer_move = (e: PointerEvent) => {
    const p = local_xy(e);
    pointers.set(e.pointerId, p);
    const xs = pointers.size > 1 ? two_touch_xs : one_touch_x;
    let xi = 0;
    for (const point of pointers.values()) {
      if (xi >= xs.length) break;
      xs[xi++] = point.x;
    }
    if (chart.native_delta_tooltip_touch_move(xs)) chart.repaint();

    const update = feed_pointer("move", e);
    if (update.kind === GestureUpdateCode.PinchMoved) {
      if (e.cancelable && update.prevent_default) e.preventDefault();
      if (chart.gesture_config().pinch_zoom && update.scale_delta !== 0) {
        // The resolver reports the fixed starting centroid and cumulative-scale difference.
        // Centroid drift never pans either scale. Pinching is direct manipulation, so it stays
        // anchored at the centroid even though wheel zoom pins the right edge.
        wasm.zoom_focused(update.x, wasm.pinch_zoom_scale(update.scale_delta));
      }
      chart.repaint();
      return;
    }
    if (e.pointerId !== active_touch_id || update.kind === GestureUpdateCode.None) return;
    if (update.kind === GestureUpdateCode.DragStarted) {
      touch_moved = true;
      clear_longpress();
      reset_tap();
    }
    if (e.cancelable && update.prevent_default) e.preventDefault();

    if (trading_dragging) {
      chart.trading_drag_to(p.y);
      set_crosshair(p.x, p.y);
    } else if (chart.creation_armed()) {
      chart.creation_pointer_move(p.x, p.y, false, false, true);
    } else if (drawing_dragging) {
      wasm.drawing_drag_to(p.x, p.y, e.ctrlKey || e.metaKey, e.shiftKey);
      set_crosshair(p.x, p.y);
    } else if (touch_tracking) {
      exit_tracking_on_next_try = false;
      if (init_crosshair !== null && track_point !== null) {
        set_crosshair(init_crosshair.x + (p.x - track_point.x), init_crosshair.y + (p.y - track_point.y));
      }
    } else if (axis_drag !== null) {
      apply_axis_drag(p);
    } else if (sep_drag !== null) {
      apply_sep_drag(p);
    } else if (touch_region === "pane") {
      if (update.kind === GestureUpdateCode.DragStarted && !touch_scrolling) {
        touch_scrolling = true;
        begin_scroll(p.x, "touch");
        start_price_pan(p.y);
      } else if (update.kind === GestureUpdateCode.DragMoved && touch_scrolling) {
        wasm.scroll_move(p.x);
        wasm.kinetic_add_sample(wasm.scroll_position(), performance.now());
        apply_price_pan(p.y);
      }
    }
    chart.repaint();
  };

  const on_touch_pointer_up = (e: PointerEvent) => {
    const p = local_xy(e);
    const update = feed_pointer("up", e);
    pointers.delete(e.pointerId);
    pointer_targets.delete(e.pointerId);
    chart.clear_trading_pressed();
    clear_longpress();

    if (update.kind === GestureUpdateCode.RebasedSinglePointer) {
      finish_scroll_without_coast();
      active_touch_id = update.pointer_id;
      pointer_targets.set(update.pointer_id, InputTargetCode.Pane);
      touch_moved = true;
      resolve_price_pan(pane_of(update.y), update.x, update.y);
      suppress_compatibility_click = true;
      chart.repaint();
      return;
    }

    if (pointers.size === 0) chart.set_interacting(false);
    if (chart.creation_capture_active()) {
      chart.creation_pointer_up(p.x, p.y, false, false);
    } else if (trading_dragging) {
      trading_dragging = false;
      chart.trading_drag_end();
    } else if (drawing_dragging) {
      drawing_dragging = false;
      wasm.drawing_drag_end();
    } else {
      end_drag("touch");
    }

    const was_tap = !touch_moved && !long_tap_active;
    const committed_on_press = creation_press_committed;
    creation_press_committed = false;
    tap_count += 1;
    if (tap_timer !== null && tap_count > 1) {
      const distance = Math.abs(e.clientX - tap_position.x) + Math.abs(e.clientY - tap_position.y);
      if (distance < DBL_TAP_MANHATTAN && was_tap && !committed_on_press) {
        // Unlike mouse compatibility events, the second tap has no preceding `click` event.
        // A variable-sequence placement receives this activation before the shared finish action.
        if (chart.creation_sequence_active()) chart.creation_click(p.x, p.y, false, false);
        else if (region_of(p, true) === "pane" && !control_click) chart.activate_drawing_double_click(p.x, p.y);
        run_dblclick(p.x, p.y);
      }
      reset_tap();
    } else if (was_tap && !committed_on_press) {
      const trading_hit = chart.trading_hit_at_device(p.x, p.y, InputDeviceCode.Touch);
      if (trading_press && trading_hit !== null) {
        control_click = true;
        chart.trading_activate_at(p.x, p.y);
      } else if (alert_press && chart.alert_create_hit_at(p.x, p.y)) {
        control_click = true;
        chart.activate_alert_create_at(p.x, p.y);
      } else if (chart.creation_armed() && chart.creation_click(p.x, p.y, false, false)) {
        // creation handled
      } else {
        chart.emit_click(p.x, p.y);
      }
    }
    if (chart.gesture_config().tracking_exit_mode === "on_touch_end") exit_tracking_on_next_try = true;
    if (touch_tracking && exit_tracking_on_next_try) {
      touch_tracking = false;
      track_point = null;
      init_crosshair = null;
      wasm.clear_crosshair();
      chart.emit_crosshair_left();
    } else if (!touch_tracking) {
      wasm.clear_crosshair();
      chart.emit_crosshair_left();
    }
    suppress_compatibility_click = true;
    if (e.cancelable) e.preventDefault();
    if (pointers.size === 0) active_touch_id = null;
    trading_press = false;
    alert_press = false;
    long_tap_active = false;
    chart.repaint();
  };

  const on_touch_pointer_cancel = (e: PointerEvent) => {
    if (e.pointerType !== "touch") return;
    suppress_compatibility_click = true;
    cancel_active_input();
  };

  /** Adapt one browser Touch to the normalized pointer-shaped sample consumed by the shared path. */
  const touch_as_pointer = (event: TouchEvent, touch: Touch): PointerEvent => ({
    pointerId: touch.identifier,
    pointerType: "touch",
    clientX: touch.clientX,
    clientY: touch.clientY,
    timeStamp: event.timeStamp,
    pressure: touch.force,
    tiltX: 0,
    tiltY: 0,
    button: 0,
    buttons: event.type === "touchend" || event.type === "touchcancel" ? 0 : 1,
    shiftKey: event.shiftKey,
    ctrlKey: event.ctrlKey,
    altKey: event.altKey,
    metaKey: event.metaKey,
    cancelable: event.cancelable,
    preventDefault: () => event.preventDefault(),
  } as unknown as PointerEvent);

  const on_touch_start = (event: TouchEvent) => {
    if (event.touches.length === 1) {
      const primary = event.touches[0]!;
      touch_origin = { x: primary.clientX, y: primary.clientY };
      touch_direction = "pending";
    } else {
      touch_direction = "chart";
    }
    for (const touch of Array.from(event.changedTouches)) {
      on_touch_pointer_down(touch_as_pointer(event, touch));
    }
    if (event.touches.length > 1 && event.cancelable) event.preventDefault();
  };

  const on_touch_move = (event: TouchEvent) => {
    if (event.touches.length > 1) {
      touch_direction = "chart";
    } else if (touch_direction === "pending" && event.touches.length === 1) {
      const primary = event.touches[0]!;
      const x_offset = Math.abs(primary.clientX - touch_origin.x);
      const y_offset = Math.abs(primary.clientY - touch_origin.y);
      if (x_offset + y_offset < SLOP_MANHATTAN) return;
      const target = pointer_targets.get(primary.identifier) ?? InputTargetCode.Pane;
      if (target !== InputTargetCode.Pane) {
        touch_direction = "chart";
      } else {
        // the public reference gives vertical movement priority by halving horizontal distance.
        const vertical = y_offset >= x_offset * 0.5;
        const cfg = chart.gesture_config();
        touch_direction = (vertical ? cfg.pan_vert_touch : cfg.pan_horz_touch) ? "chart" : "page";
      }
    }
    if (touch_direction !== "chart") return;
    if (event.cancelable) event.preventDefault();
    for (const touch of Array.from(event.changedTouches)) {
      on_touch_pointer_move(touch_as_pointer(event, touch));
    }
  };

  const on_touch_end = (event: TouchEvent) => {
    if (touch_direction === "page") {
      if (event.touches.length === 0) {
        cancel_active_input();
        touch_direction = "pending";
      }
      return;
    }
    if (event.cancelable) event.preventDefault();
    for (const touch of Array.from(event.changedTouches)) {
      on_touch_pointer_up(touch_as_pointer(event, touch));
    }
    if (event.touches.length === 0) touch_direction = "pending";
  };

  const on_touch_cancel = (event: TouchEvent) => {
    const touch = event.changedTouches[0];
    if (touch !== undefined) on_touch_pointer_cancel(touch_as_pointer(event, touch));
    else cancel_active_input();
    touch_direction = "pending";
  };

  // ---------------------------------------------------------------------------------------------
  // Keyboard
  // ---------------------------------------------------------------------------------------------

  let scroll_anim: number | null = null;
  let keyboard_pan_key: "ArrowLeft" | "ArrowRight" | null = null;
  let keyboard_pan_delta = 0;
  const stop_scroll_anim = () => {
    // A user gesture also supersedes any in-flight programmatic or keyboard scroll animation.
    chart.cancel_scroll_animation();
    wasm.cancel_keyboard_scroll();
    keyboard_pan_key = null;
    keyboard_pan_delta = 0;
    if (scroll_anim !== null) {
      cancelAnimationFrame(scroll_anim);
      scroll_anim = null;
    }
  };
  const ensure_keyboard_scroll_frames = () => {
    if (scroll_anim !== null) return;
    const step_fn = () => {
      const done = Number.isNaN(wasm.keyboard_scroll_tick(performance.now()));
      chart.repaint();
      scroll_anim = done ? null : requestAnimationFrame(step_fn);
    };
    scroll_anim = requestAnimationFrame(step_fn);
  };
  /** Keyboard pan is velocity-owned: the engine supplies low-friction repeat kicks while held and
   *  key-up cancels immediately. OS key-repeat never becomes the motion clock. */
  const begin_keyboard_scroll = (key: "ArrowLeft" | "ArrowRight", delta: number, repeat: boolean) => {
    stop_kinetic();
    // Supersede only the public scroll-to-position tween here. `chart.cancel_scroll_animation()`
    // also cancels keyboard kinetic state, which would make an OS key-repeat stop a still-held key.
    wasm.cancel_scroll_animation();
    if (chart.prefers_reduced_motion()) {
      wasm.cancel_keyboard_scroll();
      wasm.scroll_to_position(wasm.scroll_position() + delta);
      chart.repaint();
      return;
    }
    // The engine owns held-key cadence. Repeats are ignored unless a modifier changed the requested
    // speed, avoiding OS-repeat jitter while preserving live Ctrl/Shift retuning.
    if (!repeat || keyboard_pan_key !== key || keyboard_pan_delta !== delta) {
      wasm.start_keyboard_scroll(delta, performance.now());
      keyboard_pan_key = key;
      keyboard_pan_delta = delta;
    }
    ensure_keyboard_scroll_frames();
  };
  const release_keyboard_scroll = (e: KeyboardEvent) => {
    if ((e.key !== "ArrowLeft" && e.key !== "ArrowRight") || keyboard_pan_key !== e.key) return;
    e.preventDefault();
    keyboard_pan_key = null;
    keyboard_pan_delta = 0;
    wasm.cancel_keyboard_scroll();
    if (scroll_anim !== null) {
      cancelAnimationFrame(scroll_anim);
      scroll_anim = null;
    }
    chart.repaint();
    chart.announce_view();
  };

  const on_keydown = (e: KeyboardEvent) => {
    const step = e.ctrlKey || e.shiftKey ? 10 : 1;
    const center = wasm.time_scale_width() / 2;
    let handled = true;
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "z") {
      handled = e.shiftKey ? chart.redo_drawing() : chart.undo_drawing();
      if (handled) {
        e.preventDefault();
        stop_kinetic();
        chart.announce_view();
      }
      return;
    }
    switch (e.key) {
      // the public reference: Left scrolls back in time (older data), Right forward (newer data);
      // Ctrl/Shift steps 10 bars. reference rightOffset grows toward newer data, hence the signs.
      // A key whose gesture the host disabled does nothing, so its default is left alone.
      case "ArrowLeft":
        handled = chart.keyboard_time_scroll_enabled();
        if (handled) begin_keyboard_scroll("ArrowLeft", -step, e.repeat);
        break;
      case "ArrowRight":
        handled = chart.keyboard_time_scroll_enabled();
        if (handled) begin_keyboard_scroll("ArrowRight", step, e.repeat);
        break;
      case "+":
      case "=":
        handled = chart.keyboard_time_zoom_enabled();
        if (handled) wasm.zoom(center, 0.5);
        break;
      case "-":
      case "_":
        handled = chart.keyboard_time_zoom_enabled();
        if (handled) wasm.zoom(center, -0.5);
        break;
      case "Home":
        handled = chart.keyboard_time_reset_enabled();
        if (handled) wasm.fit_content();
        break;
      case "Enter":
      case "F2":
        // Enter finishes a variable sequence. Otherwise Enter and F2 edit the selected drawing's
        // text in place; the opened editor owns focus and its announcements from here.
        if (e.key === "Enter" && chart.creation_finish()) break;
        if (chart.edit_selected_drawing_text()) {
          e.preventDefault();
          stop_kinetic();
          return;
        }
        handled = false;
        break;
      case "Backspace":
        // During variable-sequence placement Backspace removes only the latest pending vertex.
        // Otherwise it retains the ordinary selected-drawing deletion behavior.
        handled = chart.creation_sequence_active() && chart.creation_active()
          ? chart.creation_pop_anchor()
          : wasm.remove_selected_drawing();
        break;
      case "Delete":
        handled = wasm.remove_selected_drawing();
        break;
      case "Escape":
        // Discard a local trading preview before clearing the remaining transient interactions.
        chart.discard_trading_interaction();
        chart.cancel_drawing_interaction();
        wasm.clear_crosshair();
        chart.repaint();
        chart.emit_crosshair_left();
        return;
      default:
        handled = false;
    }
    if (handled) {
      e.preventDefault();
      stop_kinetic();
      chart.repaint();
      chart.announce_view();
    }
  };

  const on_lost_pointer_capture = (e: PointerEvent) => {
    if (pointers.has(e.pointerId)) on_cancel(e);
  };
  const cancel_if_active = () => {
    if (pointers.size > 0) cancel_active_input();
    if (keyboard_pan_key !== null) stop_scroll_anim();
  };
  const on_visibility_change = () => {
    if (document.visibilityState !== "visible") cancel_if_active();
  };
  const gesture_resize_observer = new ResizeObserver(cancel_if_active);
  gesture_resize_observer.observe(overlay);
  set_touch_action();

  overlay.addEventListener("wheel", on_wheel, { passive: false });
  overlay.addEventListener("pointerdown", on_down);
  overlay.addEventListener("pointermove", on_move);
  overlay.addEventListener("pointerup", end_pointer);
  overlay.addEventListener("pointercancel", on_cancel);
  overlay.addEventListener("lostpointercapture", on_lost_pointer_capture);
  overlay.addEventListener("pointerleave", on_leave);
  overlay.addEventListener("click", on_click);
  overlay.addEventListener("touchstart", on_touch_start, { passive: false });
  overlay.addEventListener("touchmove", on_touch_move, { passive: false });
  overlay.addEventListener("touchend", on_touch_end, { passive: false });
  overlay.addEventListener("touchcancel", on_touch_cancel, { passive: false });
  overlay.addEventListener("contextmenu", on_contextmenu);
  overlay.addEventListener("keydown", on_keydown);
  if (is_chrome) {
    overlay.addEventListener("mousedown", on_mousedown);
  }
  // Ctrl/Cmd press/release refreshes the crosshair magnet live (reference-informed behavior).
  window.addEventListener("keydown", on_modifier_key);
  window.addEventListener("keyup", on_modifier_key);
  window.addEventListener("keyup", release_keyboard_scroll);
  window.addEventListener("blur", cancel_if_active);
  window.addEventListener("aeris_charts-chart-backend-lost", cancel_if_active);
  document.addEventListener("visibilitychange", on_visibility_change);

  return () => {
    stop_kinetic();
    stop_scroll_anim();
    clear_longpress();
    clear_tooltip_dwell();
    reset_tap();
    reset_mouse_click();
    overlay.removeEventListener("wheel", on_wheel);
    overlay.removeEventListener("pointerdown", on_down);
    overlay.removeEventListener("pointermove", on_move);
    overlay.removeEventListener("pointerup", end_pointer);
    overlay.removeEventListener("pointercancel", on_cancel);
    overlay.removeEventListener("lostpointercapture", on_lost_pointer_capture);
    overlay.removeEventListener("pointerleave", on_leave);
    overlay.removeEventListener("click", on_click);
    overlay.removeEventListener("touchstart", on_touch_start);
    overlay.removeEventListener("touchmove", on_touch_move);
    overlay.removeEventListener("touchend", on_touch_end);
    overlay.removeEventListener("touchcancel", on_touch_cancel);
    overlay.removeEventListener("contextmenu", on_contextmenu);
    overlay.removeEventListener("keydown", on_keydown);
    overlay.removeEventListener("mousedown", on_mousedown);
    window.removeEventListener("keydown", on_modifier_key);
    window.removeEventListener("keyup", on_modifier_key);
    window.removeEventListener("keyup", release_keyboard_scroll);
    window.removeEventListener("blur", cancel_if_active);
    window.removeEventListener("aeris_charts-chart-backend-lost", cancel_if_active);
    document.removeEventListener("visibilitychange", on_visibility_change);
    gesture_resize_observer.disconnect();
    cancel_if_active();
  };
}
