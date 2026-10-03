/** Browser event translation for the engine-owned chart input controller. */
import type { chart_impl } from "./impl.js";

const MODIFIER_KEYS = new Set(["Control", "Meta", "Shift", "Alt"]);

/** One pointer ABI shared by DOM and worker hosts: device | modifiers | options. */
export function controller_pointer_flags(device: number, modifiers: number, options: number): number {
  return (((options & 0xffff) << 16) | ((modifiers & 0xff) << 8) | (device & 0xff)) >>> 0;
}

type modifier_sample = { shiftKey: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean };
type point = { x: number; y: number };
const enum InputDeviceCode { Mouse = 0, Touch = 1, Pen = 2 }

export function install_gestures(chart: chart_impl): () => void {
  const overlay = chart.overlay_el();
  const wasm = chart.wasm;
  // Captures are platform resources. Gesture membership and routing stay in the engine.
  const captures = new Map<number, InputDeviceCode>();
  const controller_crosshair = new Float64Array(2);
  // DOM page-scroll arbitration waits for the engine's own drag threshold.
  const slop_manhattan = wasm.click_slop_manhattan();
  let controller_crosshair_visible = false;
  let applied_cursor: string | null = null;
  let touch_direction: "pending" | "chart" | "page" = "pending";
  let touch_origin = { x: 0, y: 0 };
  let input_wake_timer: ReturnType<typeof setTimeout> | null = null;
  let input_wake_deadline: number | null = null;
  let scroll_anim: number | null = null;
  let windows_chrome: boolean | null = null;

  const local_xy = (event: { clientX: number; clientY: number }): point => {
    const bounds = overlay.getBoundingClientRect();
    return { x: event.clientX - bounds.left - wasm.pane_left(), y: event.clientY - bounds.top };
  };
  const modifiers = (event: modifier_sample): number =>
    (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0)
      | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
  const device_code = (type: string): InputDeviceCode =>
    type === "touch" ? InputDeviceCode.Touch : type === "pen" ? InputDeviceCode.Pen : InputDeviceCode.Mouse;
  const controller_options = (): number => {
    const cfg = chart.gesture_config();
    return (cfg.pan ? 1 : 0) | (cfg.axis_scale_price ? 2 : 0)
      | (cfg.axis_scale_time ? 4 : 0) | (cfg.panes_resize ? 8 : 0)
      | (cfg.kinetic_mouse ? 16 : 0) | (chart.prefers_reduced_motion() ? 32 : 0)
      | (cfg.axis_dblclick_reset_time ? 64 : 0) | (cfg.axis_dblclick_reset_price ? 128 : 0)
      | ((cfg.pan_horz_touch || cfg.pan_vert_touch) ? 256 : 0)
      | (cfg.kinetic_touch ? 512 : 0) | (cfg.pinch_zoom ? 1024 : 0)
      | (cfg.tracking_exit_mode === "on_touch_end" ? 2048 : 0);
  };
  const flags = (device: InputDeviceCode, event: modifier_sample): number =>
    controller_pointer_flags(device, modifiers(event), controller_options());
  const cursor_names = [
    "crosshair", "default", "pointer", "text", "move", "grabbing", "grab", "grabbing",
    "ns-resize", "ew-resize", "nwse-resize", "nesw-resize", "row-resize",
  ] as const;
  const HOST_PRIMITIVE_CURSOR = 13;

  const clear_input_wake = () => {
    if (input_wake_timer !== null) clearTimeout(input_wake_timer);
    input_wake_timer = null;
    input_wake_deadline = null;
  };
  const ensure_input_frames = () => {
    if (scroll_anim !== null) return;
    const step = () => {
      if (wasm.input_tick(performance.now())) chart.repaint();
      scroll_anim = wasm.input_animating() ? requestAnimationFrame(step) : null;
    };
    scroll_anim = requestAnimationFrame(step);
  };
  const schedule_input_wake = () => {
    const deadline = wasm.input_wake_deadline_ms();
    if (deadline === undefined || !Number.isFinite(deadline)) {
      clear_input_wake();
      return;
    }
    if (input_wake_timer !== null && input_wake_deadline === deadline) return;
    clear_input_wake();
    input_wake_deadline = deadline;
    input_wake_timer = setTimeout(() => {
      input_wake_timer = null;
      input_wake_deadline = null;
      if (wasm.input_tick(performance.now())) sync_controller_pointer();
      else schedule_input_wake();
    }, Math.max(0, deadline - performance.now()));
  };
  const sync_controller_pointer = (complete_press = false) => {
    wasm.controller_crosshair_into(controller_crosshair);
    const x = controller_crosshair[0]!;
    const y = controller_crosshair[1]!;
    if (Number.isFinite(x) && Number.isFinite(y)) {
      chart.sync_hover();
      chart.emit_crosshair(x, y);
      controller_crosshair_visible = true;
    } else if (controller_crosshair_visible) {
      chart.clear_hover();
      chart.emit_crosshair_left();
      controller_crosshair_visible = false;
    }
    const cursor_code = wasm.controller_input_cursor();
    const cursor = cursor_code === HOST_PRIMITIVE_CURSOR
      ? (chart.hover_cursor() ?? "crosshair")
      : (cursor_names[cursor_code] ?? "crosshair");
    if (cursor !== applied_cursor) {
      overlay.style.cursor = cursor;
      applied_cursor = cursor;
    }
    chart.consume_input_events(complete_press);
    if (wasm.frame_pending()) chart.repaint();
    schedule_input_wake();
    if (wasm.input_animating()) ensure_input_frames();
  };
  const stop_scroll_anim = () => {
    chart.cancel_scroll_animation();
    wasm.input_cancel_motion();
    if (scroll_anim !== null) cancelAnimationFrame(scroll_anim);
    scroll_anim = null;
  };
  const cancel_active_input = () => {
    wasm.controller_pointer_cancel();
    captures.clear();
    touch_direction = "pending";
    chart.set_interacting(false);
    clear_input_wake();
    sync_controller_pointer();
  };
  const has_touch_capture = (): boolean => {
    for (const device of captures.values()) {
      if (device === InputDeviceCode.Touch) return true;
    }
    return false;
  };

  const on_down = (event: PointerEvent) => {
    if (event.pointerType === "touch" || event.button !== 0) return;
    if (has_touch_capture()) cancel_active_input();
    stop_scroll_anim();
    try { overlay.setPointerCapture(event.pointerId); } catch { /* synthetic pointer */ }
    const position = local_xy(event);
    captures.set(event.pointerId, device_code(event.pointerType));
    chart.set_interacting(true);
    wasm.controller_pointer_down(event.pointerId, position.x, position.y,
      event.timeStamp || performance.now(), event.detail || 1,
      flags(device_code(event.pointerType), event));
    sync_controller_pointer();
    if (document.getElementById("aeris_charts-text-editor") !== null) event.preventDefault();
  };
  const on_move = (event: PointerEvent) => {
    if (event.pointerType === "touch" || (event.buttons !== 0 && (event.buttons & 1) === 0)) return;
    const position = local_xy(event);
    wasm.controller_pointer_move(event.pointerId, position.x, position.y,
      event.timeStamp || performance.now(), (event.buttons & 1) !== 0,
      flags(device_code(event.pointerType), event));
    sync_controller_pointer();
  };
  const on_up = (event: PointerEvent) => {
    if (event.pointerType === "touch" || event.button !== 0) return;
    const position = local_xy(event);
    wasm.controller_pointer_up(event.pointerId, position.x, position.y,
      event.timeStamp || performance.now(), flags(device_code(event.pointerType), event));
    captures.delete(event.pointerId);
    if (captures.size === 0) chart.set_interacting(false);
    sync_controller_pointer(true);
  };
  const on_cancel = (event: PointerEvent) => {
    if (!captures.has(event.pointerId)) return;
    wasm.controller_pointer_cancel_id(event.pointerId);
    captures.delete(event.pointerId);
    if (captures.size === 0) chart.set_interacting(false);
    sync_controller_pointer();
  };
  const on_lost_pointer_capture = (event: PointerEvent) => {
    if (captures.has(event.pointerId)) on_cancel(event);
  };
  const on_leave = (event: PointerEvent) => {
    if (event.pointerType !== "mouse" || captures.size > 0) return;
    wasm.controller_pointer_leave();
    sync_controller_pointer();
  };

  const touch_flags = (event: TouchEvent): number => flags(InputDeviceCode.Touch, event);
  const on_touch_start = (event: TouchEvent) => {
    if (touch_direction === "page") return;
    if (event.touches.length === 1) {
      const first = event.touches[0]!;
      touch_origin = { x: first.clientX, y: first.clientY };
      touch_direction = "pending";
    } else {
      touch_direction = "chart";
    }
    if (captures.size > 0 && !has_touch_capture()) cancel_active_input();
    stop_scroll_anim();
    const packed = touch_flags(event);
    for (const touch of Array.from(event.changedTouches)) {
      if (captures.size >= 2) break;
      const position = local_xy(touch);
      try { overlay.setPointerCapture(touch.identifier); } catch { /* Touch Events own capture */ }
      captures.set(touch.identifier, InputDeviceCode.Touch);
      wasm.controller_pointer_down(touch.identifier, position.x, position.y,
        event.timeStamp || performance.now(), 1, packed);
    }
    if (captures.size > 0) chart.set_interacting(true);
    if (event.touches.length > 1 && event.cancelable) event.preventDefault();
    sync_controller_pointer();
  };
  const on_touch_move = (event: TouchEvent) => {
    if (touch_direction === "page") return;
    if (event.touches.length > 1) {
      touch_direction = "chart";
    } else if (touch_direction === "pending" && event.touches.length === 1) {
      const primary = event.touches[0]!;
      const dx = Math.abs(primary.clientX - touch_origin.x);
      const dy = Math.abs(primary.clientY - touch_origin.y);
      if (dx + dy < slop_manhattan) return;
      if (!wasm.controller_touch_page_scroll_candidate(primary.identifier)) {
        touch_direction = "chart";
      } else {
        // Native page scrolling gets the vertical direction priority of the public reference.
        const cfg = chart.gesture_config();
        const vertical = dy >= dx * 0.5;
        touch_direction = (vertical ? cfg.pan_vert_touch : cfg.pan_horz_touch) ? "chart" : "page";
      }
      if (touch_direction === "page") {
        wasm.controller_pointer_cancel_id(primary.identifier);
        captures.delete(primary.identifier);
        if (captures.size === 0) chart.set_interacting(false);
        sync_controller_pointer();
        return;
      }
    }
    if (touch_direction !== "chart") return;
    if (event.cancelable) event.preventDefault();
    const packed = touch_flags(event);
    for (const touch of Array.from(event.changedTouches)) {
      if (captures.get(touch.identifier) !== InputDeviceCode.Touch) continue;
      const position = local_xy(touch);
      wasm.controller_pointer_move(touch.identifier, position.x, position.y,
        event.timeStamp || performance.now(), true, packed);
    }
    sync_controller_pointer();
  };
  const on_touch_end = (event: TouchEvent) => {
    if (touch_direction === "page") {
      if (event.touches.length === 0) touch_direction = "pending";
      return;
    }
    if (event.cancelable) event.preventDefault();
    const packed = touch_flags(event);
    for (const touch of Array.from(event.changedTouches)) {
      if (captures.get(touch.identifier) !== InputDeviceCode.Touch) continue;
      const position = local_xy(touch);
      wasm.controller_pointer_up(touch.identifier, position.x, position.y,
        event.timeStamp || performance.now(), packed);
      captures.delete(touch.identifier);
    }
    if (captures.size === 0) chart.set_interacting(false);
    if (event.touches.length === 0) touch_direction = "pending";
    sync_controller_pointer(true);
  };
  const on_touch_cancel = (_event: TouchEvent) => cancel_active_input();

  const on_wheel = (event: WheelEvent) => {
    const cfg = chart.gesture_config();
    const position = local_xy(event);
    if (windows_chrome === null) {
      const nav = navigator as Navigator & {
        userAgentData?: { platform?: string; brands?: { brand: string }[] };
      };
      const chromium = nav.userAgentData?.brands?.some((brand) => brand.brand.includes("Chromium")) === true;
      const windows = nav.userAgentData?.platform
        ? nav.userAgentData.platform === "Windows"
        : navigator.userAgent.toLowerCase().includes("win");
      windows_chrome = chromium && windows;
    }
    const pixel_ratio = windows_chrome ? window.devicePixelRatio : 1;
    const behavior = cfg.wheel_behavior === "pan" ? 1 : cfg.wheel_behavior === "zoom" ? 2 : 0;
    const consumed = wasm.input_wheel(position.x, position.y, event.deltaX, event.deltaY,
      event.deltaMode, pixel_ratio, modifiers(event), behavior, cfg.wheel_scroll, cfg.wheel_zoom,
      cfg.price_axis_wheel_zoom, event.timeStamp || performance.now());
    if (!consumed) return;
    if (event.cancelable) event.preventDefault();
    sync_controller_pointer();
  };
  const on_contextmenu = (event: MouseEvent) => {
    const position = local_xy(event);
    const handled = chart.has_chart_context_subscribers();
    wasm.controller_context_menu(position.x, position.y);
    sync_controller_pointer();
    if (handled) event.preventDefault();
  };
  const on_mousedown = (event: MouseEvent) => {
    if (event.button === 1) event.preventDefault();
  };
  const on_modifier_key = (event: KeyboardEvent) => {
    if (event.key !== "Control" && event.key !== "Meta") return;
    wasm.input_modifiers_changed(modifiers(event));
    sync_controller_pointer();
  };
  const on_keydown = (event: KeyboardEvent) => {
    if (!wasm.input_key_down(event.key, modifiers(event), event.repeat, event.timeStamp,
      chart.keyboard_gates())) return;
    if (!MODIFIER_KEYS.has(event.key)) event.preventDefault();
    chart.consume_input_events();
    chart.repaint();
    chart.announce_view();
    if (wasm.input_animating()) ensure_input_frames();
  };
  const on_keyup = (event: KeyboardEvent) => {
    if (!wasm.input_key_up(event.key, modifiers(event))) return;
    if (!MODIFIER_KEYS.has(event.key)) event.preventDefault();
    chart.repaint();
    chart.announce_view();
  };
  const cancel_if_active = () => {
    if (captures.size > 0) cancel_active_input();
    if (wasm.input_animating()) stop_scroll_anim();
    clear_input_wake();
  };
  const on_visibility_change = () => {
    if (document.visibilityState !== "visible") cancel_if_active();
  };
  const gesture_resize_observer = new ResizeObserver(cancel_if_active);
  gesture_resize_observer.observe(overlay);
  overlay.style.touchAction = "auto";

  overlay.addEventListener("wheel", on_wheel, { passive: false });
  overlay.addEventListener("pointerdown", on_down);
  overlay.addEventListener("pointermove", on_move);
  overlay.addEventListener("pointerup", on_up);
  overlay.addEventListener("pointercancel", on_cancel);
  overlay.addEventListener("lostpointercapture", on_lost_pointer_capture);
  overlay.addEventListener("pointerleave", on_leave);
  overlay.addEventListener("touchstart", on_touch_start, { passive: false });
  overlay.addEventListener("touchmove", on_touch_move, { passive: false });
  overlay.addEventListener("touchend", on_touch_end, { passive: false });
  overlay.addEventListener("touchcancel", on_touch_cancel, { passive: false });
  overlay.addEventListener("contextmenu", on_contextmenu);
  overlay.addEventListener("keydown", on_keydown);
  if ((window as Window & { chrome?: unknown }).chrome !== undefined) {
    overlay.addEventListener("mousedown", on_mousedown);
  }
  window.addEventListener("keydown", on_modifier_key);
  window.addEventListener("keyup", on_modifier_key);
  window.addEventListener("keyup", on_keyup);
  window.addEventListener("blur", cancel_if_active);
  window.addEventListener("aeris_charts-chart-backend-lost", cancel_if_active);
  document.addEventListener("visibilitychange", on_visibility_change);

  return () => {
    stop_scroll_anim();
    clear_input_wake();
    overlay.removeEventListener("wheel", on_wheel);
    overlay.removeEventListener("pointerdown", on_down);
    overlay.removeEventListener("pointermove", on_move);
    overlay.removeEventListener("pointerup", on_up);
    overlay.removeEventListener("pointercancel", on_cancel);
    overlay.removeEventListener("lostpointercapture", on_lost_pointer_capture);
    overlay.removeEventListener("pointerleave", on_leave);
    overlay.removeEventListener("touchstart", on_touch_start);
    overlay.removeEventListener("touchmove", on_touch_move);
    overlay.removeEventListener("touchend", on_touch_end);
    overlay.removeEventListener("touchcancel", on_touch_cancel);
    overlay.removeEventListener("contextmenu", on_contextmenu);
    overlay.removeEventListener("keydown", on_keydown);
    overlay.removeEventListener("mousedown", on_mousedown);
    window.removeEventListener("keydown", on_modifier_key);
    window.removeEventListener("keyup", on_modifier_key);
    window.removeEventListener("keyup", on_keyup);
    window.removeEventListener("blur", cancel_if_active);
    window.removeEventListener("aeris_charts-chart-backend-lost", cancel_if_active);
    document.removeEventListener("visibilitychange", on_visibility_change);
    gesture_resize_observer.disconnect();
    cancel_active_input();
  };
}
