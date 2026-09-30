import { test, expect } from "@playwright/test";

async function create_worker_chart(page, backend, init_extra = {}, device_dpr = false) {
  await page.goto("/");
  const supported = await page.evaluate(() =>
    typeof OffscreenCanvas !== "undefined"
    && "transferControlToOffscreen" in HTMLCanvasElement.prototype,
  );
  test.skip(!supported, "OffscreenCanvas transfer is unavailable in this browser");

  await page.evaluate(({ requested_backend, init_extra, device_dpr }) => {
    const host = document.createElement("div");
    host.id = "offscreen-worker-host";
    host.style.cssText = "position:fixed;left:0;top:0;width:640px;height:360px;z-index:1000;background:white";
    const gpu = document.createElement("canvas");
    const fallback = document.createElement("canvas");
    for (const canvas of [gpu, fallback]) {
      canvas.style.cssText = "position:absolute;inset:0;width:640px;height:360px";
      host.appendChild(canvas);
    }
    document.body.appendChild(host);
    const worker = new Worker("/offscreen_chart_worker.js", { type: "module" });
    const messages = [];
    worker.onmessage = (event) => {
      messages.push(event.data);
      if (event.data.backend) {
        gpu.style.visibility = event.data.backend === "webgpu" ? "visible" : "hidden";
        fallback.style.visibility = event.data.backend === "canvas2d" ? "visible" : "hidden";
      }
    };
    worker.onerror = (event) => messages.push({ type: "error", message: event.message });
    const gpu_canvas = gpu.transferControlToOffscreen();
    const fallback_canvas = fallback.transferControlToOffscreen();
    worker.postMessage({
      type: "init",
      gpu_canvas,
      fallback_canvas,
      width: 640,
      height: 360,
      dpr: device_dpr ? window.devicePixelRatio : 1,
      backend: requested_backend,
      bars: 5_000,
      force_fallback_adapter: true,
      ...init_extra,
    }, [gpu_canvas, fallback_canvas]);
    window.__offscreen_worker = { worker, messages };
  }, { requested_backend: backend, init_extra, device_dpr });

  await page.waitForFunction(() => window.__offscreen_worker.messages.some((message) =>
    message.type === "ready" || message.type === "error",
  ));
  const ready = await page.evaluate(() => window.__offscreen_worker.messages.at(-1));
  expect(ready.type, ready.message).toBe("ready");
  return ready;
}

async function send(page, message) {
  const count = await page.evaluate((payload) => {
    const state = window.__offscreen_worker;
    const before = state.messages.length;
    state.worker.postMessage(payload);
    return before;
  }, message);
  await page.waitForFunction((before) => window.__offscreen_worker.messages.length > before, count);
  const result = await page.evaluate(() => window.__offscreen_worker.messages.at(-1));
  expect(result.type, result.message).not.toBe("error");
  return result;
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
});

test.afterEach(async ({ page }) => {
  await page.evaluate(() => window.__offscreen_worker?.worker.terminate());
});

test("OffscreenCanvas worker renders, resizes, and accepts relayed pointer/wheel/key input", async ({ page }) => {
  test.setTimeout(120_000);
  const ready = await create_worker_chart(page, undefined);
  expect(ready.backend).toBe("webgpu");
  expect(ready.stats.presented_frames).toBeGreaterThan(0);
  expect(ready.stats.draw_calls).toBeGreaterThan(0);
  expect(ready.stats.canvas2d_ops).toBe(0);
  expect(ready.range).not.toBeNull();
  expect(ready.size).toEqual([640, 360]);

  const wheel = await send(page, {
    type: "wheel",
    event: { x: 320, y: 160, delta_x: 0, delta_y: -120, delta_mode: 0 },
  });
  expect(wheel.range).not.toEqual(ready.range);
  expect(wheel.stats.canvas2d_ops).toBe(0);

  await send(page, { type: "pointer", event: { type: "down", x: 250, y: 150, pointer_id: 7 } });
  const dragged = await send(page, {
    type: "pointer",
    event: { type: "move", x: 390, y: 155, pointer_id: 7, buttons: 1 },
  });
  await send(page, { type: "pointer", event: { type: "up", x: 390, y: 155, pointer_id: 7 } });
  expect(dragged.range).not.toEqual(wheel.range);

  // A second pointer may be active, but canceling the pointer that owns the drag must end the
  // scroll session; the non-owner cannot inherit it.
  await send(page, { type: "pointer", event: { type: "down", x: 260, y: 150, pointer_id: 8, pointer_type: "touch" } });
  await send(page, { type: "pointer", event: { type: "down", x: 320, y: 150, pointer_id: 9, pointer_type: "touch" } });
  const pinched = await send(page, {
    type: "pointer", event: { type: "move", x: 380, y: 165, pointer_id: 9, pointer_type: "touch", buttons: 1 },
  });
  expect(pinched.range).not.toEqual(dragged.range);
  const canceled = await send(page, {
    type: "pointer", event: { type: "cancel", x: 260, y: 150, pointer_id: 8, pointer_type: "touch" },
  });
  const non_owner_move = await send(page, {
    type: "pointer", event: { type: "move", x: 500, y: 150, pointer_id: 9, pointer_type: "touch", buttons: 1 },
  });
  expect(non_owner_move.range).toEqual(canceled.range);
  await send(page, { type: "pointer", event: { type: "up", x: 500, y: 150, pointer_id: 9, pointer_type: "touch" } });

  const keyed = await send(page, { type: "key", event: { key: "ArrowRight" } });
  expect(keyed.range).not.toEqual(dragged.range);

  const resized = await send(page, { type: "resize", width: 500, height: 300, dpr: 2 });
  expect(resized.size).toEqual([1_000, 600]);
  expect(resized.stats.presented_frames).toBeGreaterThan(keyed.stats.presented_frames);
});

test("worker wheel routing matches the main-thread gesture router", async ({ page }) => {
  test.setTimeout(120_000);
  const ready = await create_worker_chart(page, undefined, {}, true);
  // A main-thread twin with the worker fixture's exact data, size, and device pixel ratio.
  const twin_range = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText =
      "position:fixed;left:0;top:380px;width:640px;height:360px;z-index:1000;background:white";
    document.body.append(host);
    const chart = await create_chart(host, { autoSize: false, backend: "canvas2d" });
    chart.resize(640, 360);
    const count = 5_000;
    const columns = {
      times: new Float64Array(count),
      open: new Float64Array(count),
      high: new Float64Array(count),
      low: new Float64Array(count),
      close: new Float64Array(count),
    };
    let price = 100;
    for (let i = 0; i < count; i += 1) {
      const next = price + Math.sin(i * 0.037) * 0.45;
      columns.times[i] = 1_577_836_800 + i * 60;
      columns.open[i] = price;
      columns.high[i] = Math.max(price, next) + 0.2;
      columns.low[i] = Math.min(price, next) - 0.2;
      columns.close[i] = next;
      price = next;
    }
    chart.add_series("candlestick").set_data_typed(columns);
    chart.time_scale().fit_content();
    window.__wheel_twin = { chart, host };
    return chart.time_scale().get_visible_logical_range();
  });
  expect(twin_range.from).toBeCloseTo(ready.range.from, 9);
  expect(twin_range.to).toBeCloseTo(ready.range.to, 9);

  const samples = [
    // Auto mode: vertical wheel over the price axis zooms time, not price.
    { x: 628, y: 150, delta_x: 0, delta_y: -120, delta_mode: 0 },
    // Auto mode ignores Ctrl and Shift.
    { x: 200, y: 150, delta_x: 0, delta_y: -120, delta_mode: 0, ctrl_key: true },
    { x: 300, y: 150, delta_x: 0, delta_y: 120, delta_mode: 0, shift_key: true },
    { x: 320, y: 150, delta_x: 80, delta_y: 0, delta_mode: 0 },
    { x: 420, y: 150, delta_x: 0, delta_y: -3, delta_mode: 1 },
  ];
  let previous = ready.range;
  for (const sample of samples) {
    const worker_state = await send(page, { type: "wheel", event: sample });
    const main_range = await page.evaluate((sample) => {
      const { chart } = window.__wheel_twin;
      const overlay = chart.chart_element().querySelector("canvas:last-of-type");
      const rect = overlay.getBoundingClientRect();
      overlay.dispatchEvent(new WheelEvent("wheel", {
        deltaX: sample.delta_x,
        deltaY: sample.delta_y,
        deltaMode: sample.delta_mode,
        ctrlKey: sample.ctrl_key === true,
        shiftKey: sample.shift_key === true,
        clientX: rect.left + sample.x,
        clientY: rect.top + sample.y,
        bubbles: true,
        cancelable: true,
      }));
      return chart.time_scale().get_visible_logical_range();
    }, sample);
    expect(worker_state.range, JSON.stringify(sample)).not.toEqual(previous);
    expect(worker_state.range.from, JSON.stringify(sample)).toBeCloseTo(main_range.from, 9);
    expect(worker_state.range.to, JSON.stringify(sample)).toBeCloseTo(main_range.to, 9);
    previous = worker_state.range;
  }
  await page.evaluate(() => {
    window.__wheel_twin.chart.remove();
    window.__wheel_twin.host.remove();
  });
});

test("worker frames continue while the main thread is blocked for 500 ms", async ({ page }) => {
  test.setTimeout(120_000);
  const ready = await create_worker_chart(page, undefined);
  const started = await send(page, { type: "start", width: 640, height: 360 });

  await page.evaluate(() => {
    const until = performance.now() + 550;
    while (performance.now() < until) {
      // Deliberately occupy the window event loop; the dedicated worker must keep presenting.
    }
  });

  const stopped = await send(page, { type: "stop" });
  console.log(`offscreen blocked-main result: ${JSON.stringify({
    worker_frames: stopped.frame - started.frame,
    presented_frames: stopped.stats.presented_frames - started.stats.presented_frames,
    cpu_ms: stopped.stats.cpu_ms,
  })}`);
  expect(stopped.frame - started.frame).toBeGreaterThanOrEqual(10);
  expect(stopped.stats.presented_frames - started.stats.presented_frames).toBeGreaterThanOrEqual(10);
  expect(stopped.stats.dropped_frames).toBe(ready.stats.dropped_frames);
});

test("runtime WebGPU loss notifies the owner to reveal the warm Canvas2D surface", async ({ page }) => {
  const ready = await create_worker_chart(page, undefined);
  expect(ready.backend).toBe("webgpu");

  const changed = await send(page, { type: "simulate_loss" });
  expect(changed.type).toBe("backend_change");
  expect(changed.backend).toBe("canvas2d");
  expect(changed.stats.canvas2d_ops).toBeGreaterThan(0);
  const visibility = await page.evaluate(() => {
    const [gpu, fallback] = document.querySelectorAll("#offscreen-worker-host canvas");
    return [gpu.style.visibility, fallback.style.visibility];
  });
  expect(visibility).toEqual(["hidden", "visible"]);
});

test("OffscreenCanvas keeps the Canvas2D fallback path renderable", async ({ page }) => {
  const ready = await create_worker_chart(page, "canvas2d");
  expect(ready.backend).toBe("canvas2d");
  expect(ready.stats.presented_frames).toBeGreaterThan(0);
  expect(ready.stats.canvas2d_ops).toBeGreaterThan(0);
  const moved = await send(page, {
    type: "pointer",
    event: { type: "move", x: 300, y: 140, pointer_id: 1 },
  });
  expect(moved.stats.presented_frames).toBeGreaterThan(ready.stats.presented_frames);
  expect(moved.stats.canvas2d_ops).toBeGreaterThan(0);
});

test("OffscreenCanvas exposes atomic timestamp rejection diagnostics", async ({ page }) => {
  await create_worker_chart(page, "canvas2d");
  const result = await send(page, { type: "invalid_timestamp" });
  expect(result.type).toBe("timestamp_diagnostics");
  expect(result.diagnostics).toMatchObject({ status: "rejected", accepted: 0 });
  expect(result.diagnostics.reason).toContain("milliseconds");
});

test("worker charts resolve a declarative IANA time zone and session start", async ({ page }) => {
  test.setTimeout(120_000);
  // Construction-time declarative option (IANA name resolved with the worker's own Intl).
  await create_worker_chart(page, "canvas2d", { time_scale: { timeZone: "Asia/Shanghai" } });
  const probe = 1_704_159_000; // 2024-01-02 01:30 UTC = 09:30 CST
  const initial = await send(page, { type: "time_zone", probe });
  expect(initial.type).toBe("time_zone");
  expect(Array.isArray(initial.time_zone)).toBe(true);
  expect(initial.time_zone.at(-1).offset_seconds).toBe(28_800);
  expect(initial.local - probe).toBe(8 * 3_600);

  // Runtime apply_options with a DST zone and a session start.
  const eastern = await send(page, {
    type: "time_zone",
    probe: 1_720_000_000, // July 2024: EDT
    options: { timeScale: { timeZone: "America/New_York", sessionStart: -6 * 3_600 } },
  });
  expect(eastern.local - 1_720_000_000).toBe(-4 * 3_600);
  expect(eastern.session_start).toBe(-21_600);
  expect(Array.isArray(eastern.time_zone)).toBe(true);
  expect(eastern.time_zone.length).toBeGreaterThan(100);

  // An unknown zone is rejected without changing the installed zone.
  const rejected = await send(page, {
    type: "time_zone",
    options: { timeScale: { timeZone: "Nowhere/Invalid" } },
  });
  expect(rejected).toMatchObject({ type: "time_zone_error", code: "invalid_options" });
  const unchanged = await send(page, { type: "time_zone", probe: 1_720_000_000 });
  expect(unchanged.local - 1_720_000_000).toBe(-4 * 3_600);
});

test("worker charts accept declarative explicit time-axis marks", async ({ page }) => {
  await create_worker_chart(page, "canvas2d");
  const marks = await send(page, {
    type: "time_zone",
    probe: 1_704_159_000,
    options: { timeScale: { tickMarks: [{ time: "2024-01-02", label: "Jan 2" }, { time: 1_704_240_000 }] } },
  });
  expect(marks.tick_marks).toEqual([{ time: 1_704_153_600, label: "Jan 2" }, { time: 1_704_240_000 }]);
  // An unordered list is rejected without changing the installed marks.
  const rejected = await send(page, {
    type: "time_zone",
    options: { timeScale: { tickMarks: [{ time: 2 }, { time: 1 }] } },
  });
  expect(rejected).toMatchObject({ type: "time_zone_error", code: "invalid_options" });
  const unchanged = await send(page, { type: "time_zone", probe: 1_704_159_000 });
  expect(unchanged.tick_marks).toEqual(marks.tick_marks);
  const cleared = await send(page, {
    type: "time_zone",
    probe: 1_704_159_000,
    options: { timeScale: { tickMarks: null } },
  });
  expect(cleared.tick_marks).toBeNull();
});

test("worker charts accept a declarative bar time label and paint the close on the axis", async ({ page }) => {
  test.setTimeout(120_000);
  await create_worker_chart(page, "canvas2d");
  const probe = 1_704_159_000; // the worker fixture stamps minute bars 60 s apart
  // Crosshair text of the frame the worker's own engine paints: the date and the wall-clock time.
  const crosshair_minutes = async () => {
    const { texts } = await send(page, { type: "axis_texts", x: 320, y: 120 });
    const label = texts.find((text) => /^\d\d \w{3} '\d\d\s+\d\d:\d\d$/.test(text));
    expect(label, `crosshair text among ${JSON.stringify(texts)}`).toBeDefined();
    const [hours, minutes] = label.slice(-5).split(":").map(Number);
    return hours * 60 + minutes;
  };
  const shown = await send(page, { type: "time_zone", probe, options: { timeScale: { timeVisible: true } } });
  expect(shown.bar_time_label).toBe("open");
  expect(shown.printed).toBe(probe);
  const open_minute = await crosshair_minutes();

  // A label-only patch (no zone, session start, or marks) must reach the engine.
  const closed = await send(page, {
    type: "time_zone",
    probe,
    options: { timeScale: { barTimeLabel: { anchor: "close", interval_seconds: 60 } } },
  });
  expect(closed.bar_time_label).toMatchObject({ anchor: "close", interval_seconds: 60, windows: [] });
  expect(closed.printed).toBe(probe + 60);
  expect((await crosshair_minutes() - open_minute + 1_440) % 1_440).toBe(1);

  // An invalid label rejects the whole patch and keeps the installed one.
  const rejected = await send(page, {
    type: "time_zone",
    options: { timeScale: { barTimeLabel: { anchor: "close", interval_seconds: 0 }, timeVisible: false } },
  });
  expect(rejected).toMatchObject({ type: "time_zone_error", code: "invalid_options" });
  const unchanged = await send(page, { type: "time_zone", probe });
  expect(unchanged.bar_time_label).toMatchObject({ anchor: "close", interval_seconds: 60 });

  // Windows validate together with the zone and session start of the same patch.
  const windowed = await send(page, {
    type: "time_zone",
    probe,
    options: {
      timeScale: {
        timeZone: "Asia/Shanghai",
        barTimeLabel: { anchor: "close", interval_seconds: 3_600, windows: [["09:30", "11:30"], ["13:00", "15:00"]] },
      },
    },
  });
  expect(windowed.bar_time_label.windows).toEqual([["09:30", "11:30"], ["13:00", "15:00"]]);
  const opened = await send(page, { type: "time_zone", probe, options: { timeScale: { barTimeLabel: "open" } } });
  expect(opened.bar_time_label).toBe("open");
  expect(opened.printed).toBe(probe);
  // Back to the open text, now in Shanghai time: the same bar, eight hours later on the clock.
  expect(await crosshair_minutes()).toBe((open_minute + 480) % 1_440);
});

test("OffscreenCanvas typed streaming honors the sequence guard and merges partial columns", async ({ page }) => {
  await create_worker_chart(page, "canvas2d");
  const result = await send(page, { type: "sequenced_stream", bars: 5_000 });
  expect(result.type).toBe("sequenced_stream");
  expect(result.results.applied).toBeNull();
  expect(result.results.stale).toMatchObject({ status: "rejected", code: "stale_sequence", last_sequence: 3 });
  expect(result.results.merged).toBeNull();
  expect(result.results.merge_stale).toMatchObject({ status: "rejected", code: "stale_sequence", last_sequence: 4 });
  expect(result.results.empty).toMatchObject({ status: "rejected", code: "empty_merge" });
});
