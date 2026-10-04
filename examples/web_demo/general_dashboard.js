import { create_chart } from "./dist/aeris_charts_financial.js";

let dashboard_root = null;
let chart_grid = null;
let metric_ready = null;
let runtime_badge = null;
let dashboard_error = null;
let requested_backend = "auto";
let active_theme = "dark";
const MAX_ACTIVE_CHARTS = 4;

const palette = {
  blue: "#4c8bf5",
  violet: "#8b6df6",
  teal: "#28b7a4",
  amber: "#e4a11b",
  coral: "#e46f61",
  cyan: "#25a8c7",
};

const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const month_rows = (values) => values.map((y, index) => ({ id: index, x: months[index], y }));
const axes = (x_scale, x_title, y_title) => [
  { id: "x", dimension: "x", scale: x_scale, title: x_title, grid_visible: false },
  { id: "y", dimension: "y", scale: "linear", title: y_title, grid_visible: true },
];

const examples = [
  {
    title: "Revenue trend", description: "A category-point line tracks monthly revenue across the year.", label: "xy_line", category: "trend",
    horizontal_domain: { type: "category", scale: "point" }, axes: axes("point", "Month", "Revenue"),
    series: [{ kind: "xy_line", title: "Revenue", color: palette.blue, data: month_rows([42, 47, 45, 54, 58, 61, 66, 64, 72, 77, 81, 88]) }],
  },
  {
    title: "Margin area", description: "A filled area demonstrates ordered categorical trends and automatic Y scaling.", label: "xy_area", category: "trend",
    horizontal_domain: { type: "category", scale: "point" }, axes: axes("point", "Month", "Margin"),
    series: [{ kind: "xy_area", title: "Margin", color: palette.teal, data: month_rows([22, 24, 23, 27, 29, 31, 33, 32, 35, 37, 39, 42]) }],
  },
  {
    title: "Channel revenue", description: "Standard columns show a compact category comparison with data labels.", label: "column", category: "bar",
    horizontal_domain: { type: "category", scale: "band" }, axes: axes("band", "Channel", "Revenue"),
    series: [{ kind: "column", title: "Revenue", color: palette.blue, data_labels: true, data: [
      { x: "Direct", y: 74 }, { x: "Partner", y: 51 }, { x: "Market", y: 38 }, { x: "Retail", y: 62 }, { x: "API", y: 83 },
    ] }],
  },
  {
    title: "Regional comparison", description: "Grouped columns place two series side by side inside each category band.", label: "grouped columns", category: "bar",
    horizontal_domain: { type: "category", scale: "band" }, axes: axes("band", "Quarter", "Bookings"),
    series: [
      { kind: "column", title: "North", color: palette.blue, group_id: "regions", data: [
        { x: "Q1", y: 48 }, { x: "Q2", y: 57 }, { x: "Q3", y: 63 }, { x: "Q4", y: 71 },
      ] },
      { kind: "column", title: "South", color: palette.violet, group_id: "regions", data: [
        { x: "Q1", y: 39 }, { x: "Q2", y: 46 }, { x: "Q3", y: 52 }, { x: "Q4", y: 65 },
      ] },
    ],
  },
  {
    title: "Revenue mix", description: "Three series accumulate into a normal stack for each quarter.", label: "stacked columns", category: "bar",
    horizontal_domain: { type: "category", scale: "band" }, axes: axes("band", "Quarter", "Revenue"),
    series: [
      { kind: "column", title: "Core", color: palette.blue, group_id: "mix", stack_id: "revenue", stack_mode: "normal", data: [
        { x: "Q1", y: 34 }, { x: "Q2", y: 39 }, { x: "Q3", y: 43 }, { x: "Q4", y: 47 },
      ] },
      { kind: "column", title: "Growth", color: palette.teal, group_id: "mix", stack_id: "revenue", stack_mode: "normal", data: [
        { x: "Q1", y: 13 }, { x: "Q2", y: 16 }, { x: "Q3", y: 21 }, { x: "Q4", y: 25 },
      ] },
      { kind: "column", title: "Services", color: palette.amber, group_id: "mix", stack_id: "revenue", stack_mode: "normal", data: [
        { x: "Q1", y: 9 }, { x: "Q2", y: 11 }, { x: "Q3", y: 12 }, { x: "Q4", y: 14 },
      ] },
    ],
  },
  {
    title: "Pipeline by stage", description: "Horizontal bars bind numeric X to a category Y axis for ranked comparisons.", label: "horizontal_bar", category: "bar",
    horizontal_domain: { type: "continuous", scale: "linear" },
    axes: [
      { id: "x", dimension: "x", scale: "linear", title: "Pipeline", grid_visible: true },
      { id: "y", dimension: "y", scale: "band", title: "Stage", grid_visible: false },
    ],
    series: [{ kind: "horizontal_bar", title: "Pipeline", color: palette.violet, data_labels: true, data: [
      { x: "Qualified", y: 82 }, { x: "Discovery", y: 67 }, { x: "Proposal", y: 49 }, { x: "Security", y: 36 }, { x: "Contract", y: 24 },
    ] }],
  },
  {
    title: "Spend efficiency", description: "Independent numeric axes show the relationship between spend and revenue.", label: "scatter", category: "distribution",
    horizontal_domain: { type: "continuous", scale: "linear" }, axes: axes("linear", "Spend", "Revenue"),
    series: [{ kind: "scatter", title: "Campaigns", color: palette.cyan, point_radius: 5, data: [
      { x: 12, y: 36 }, { x: 17, y: 41 }, { x: 19, y: 52 }, { x: 24, y: 58 }, { x: 28, y: 69 },
      { x: 31, y: 65 }, { x: 36, y: 78 }, { x: 42, y: 88 }, { x: 45, y: 81 }, { x: 51, y: 96 },
    ] }],
  },
  {
    title: "Market opportunity", description: "Bubble size adds a third quantitative channel to an XY comparison.", label: "bubble", category: "distribution",
    horizontal_domain: { type: "continuous", scale: "linear" }, axes: axes("linear", "Adoption", "Growth"),
    series: [{ kind: "bubble", title: "Segments", color: palette.violet, data: [
      { x: 72, y: 38, size: 196, label: "Enterprise" }, { x: 54, y: 57, size: 144, label: "Mid-market" },
      { x: 36, y: 74, size: 100, label: "SMB" }, { x: 24, y: 88, size: 64, label: "Startup" }, { x: 81, y: 24, size: 81, label: "Public sector" },
    ] }],
  },
  {
    title: "Forecast band", description: "Low/high bounds form a confidence band over an ordered numeric horizon.", label: "range_area", category: "trend",
    horizontal_domain: { type: "continuous", scale: "linear" }, axes: axes("linear", "Week", "Forecast"),
    series: [{ kind: "range_area", title: "80% interval", color: palette.teal, data: [
      { x: 1, low: 42, high: 57 }, { x: 2, low: 44, high: 61 }, { x: 3, low: 48, high: 66 }, { x: 4, low: 51, high: 70 },
      { x: 5, low: 55, high: 75 }, { x: 6, low: 58, high: 79 }, { x: 7, low: 62, high: 84 }, { x: 8, low: 66, high: 89 },
    ] }],
  },
  {
    title: "Delivery windows", description: "Category range bars show low/high delivery estimates as exact rectangular marks.", label: "range_bar", category: "bar",
    horizontal_domain: { type: "category", scale: "band" },
    axes: [
      { id: "x", dimension: "x", scale: "band", title: "Team", grid_visible: false },
      { id: "y", dimension: "y", scale: "linear", title: "Days", grid_visible: true },
    ],
    series: [{ kind: "range_bar", title: "Delivery", color: palette.amber, data: [
      { x: "Core", low: 4, high: 9 }, { x: "Growth", low: 7, high: 13 },
      { x: "Platform", low: 3, high: 8 }, { x: "Data", low: 9, high: 16 },
    ] }],
  },
  {
    title: "Experiment uncertainty", description: "Category-centered error bars show measured values with Y confidence bounds.", label: "error_bar", category: "distribution",
    horizontal_domain: { type: "category", scale: "point" }, axes: axes("point", "Variant", "Conversion"),
    series: [{ kind: "error_bar", title: "Conversion", color: palette.coral, point_radius: 6, data: [
      { x: "Control", y: 4.8, y_low: 4.2, y_high: 5.4 }, { x: "A", y: 5.7, y_low: 5.0, y_high: 6.5 },
      { x: "B", y: 6.3, y_low: 5.5, y_high: 7.1 }, { x: "C", y: 5.4, y_low: 4.8, y_high: 6.0 },
    ] }],
  },
  {
    title: "Latency distribution", description: "Five-number summaries render through the box-plot geometry path.", label: "box_plot", category: "distribution",
    horizontal_domain: { type: "category", scale: "band" }, axes: axes("band", "Region", "Milliseconds"),
    series: [{ kind: "box_plot", title: "Latency", color: palette.blue, data: [
      { x: "US-E", min: 18, q1: 23, median: 29, q3: 37, max: 52 }, { x: "US-W", min: 22, q1: 28, median: 34, q3: 41, max: 58 },
      { x: "EU", min: 31, q1: 38, median: 45, q3: 55, max: 72 }, { x: "APAC", min: 44, q1: 53, median: 62, q3: 75, max: 96 },
    ] }],
  },
  {
    title: "Engagement heatmap", description: "A weekday × hour grid exercises two categorical dimensions and dense cells.", label: "heatmap_grid", category: "dense",
    horizontal_domain: { type: "category", scale: "band" },
    axes: [
      { id: "x", dimension: "x", scale: "band", title: "Hour", grid_visible: false },
      { id: "y", dimension: "y", scale: "band", title: "Day", grid_visible: false },
    ],
    series: [{ kind: "heatmap_grid", title: "Sessions", color: palette.cyan, data: (() => {
      const days = ["Mon", "Tue", "Wed", "Thu", "Fri"];
      const hours = ["09", "11", "13", "15", "17"];
      return days.flatMap((day, d) => hours.map((hour, h) => ({
        id: `${day}-${hour}`, x: hour, y: day, value: 18 + d * 7 + h * 9 + ((d + h) % 3) * 6,
      })));
    })() }],
  },
  {
    title: "Portfolio composition", description: "Percent-stacked areas align by category identity and normalize each quarter.", label: "stacked area", category: "trend",
    horizontal_domain: { type: "category", scale: "point" }, axes: axes("point", "Quarter", "Share"),
    series: [
      { kind: "xy_area", title: "Core", color: palette.blue, stack_id: "portfolio", stack_mode: "percent", data: [
        { x: "Q1", y: 52 }, { x: "Q2", y: 49 }, { x: "Q3", y: 46 }, { x: "Q4", y: 43 },
      ] },
      { kind: "xy_area", title: "Growth", color: palette.teal, stack_id: "portfolio", stack_mode: "percent", data: [
        { x: "Q1", y: 28 }, { x: "Q2", y: 31 }, { x: "Q3", y: 34 }, { x: "Q4", y: 36 },
      ] },
      { kind: "xy_area", title: "New", color: palette.amber, stack_id: "portfolio", stack_mode: "percent", data: [
        { x: "Q1", y: 20 }, { x: "Q2", y: 20 }, { x: "Q3", y: 20 }, { x: "Q4", y: 21 },
      ] },
    ],
  },
];

function tooltip_text(series, row) {
  const snapshot = series.data_at(row);
  if (snapshot === null) return null;
  const detail = [];
  if (snapshot.value !== null) detail.push(`value ${Number(snapshot.value).toLocaleString()}`);
  if (snapshot.low !== null || snapshot.high !== null) detail.push(`range ${snapshot.low ?? "—"}–${snapshot.high ?? "—"}`);
  if (snapshot.q1 !== null || snapshot.q3 !== null) detail.push(`q1 ${snapshot.q1 ?? "—"} · q3 ${snapshot.q3 ?? "—"}`);
  if (snapshot.size !== null) detail.push(`size ${snapshot.size}`);
  if (snapshot.y_label !== null) detail.push(`y ${snapshot.y_label}`);
  return { title: snapshot.label ?? snapshot.x_label ?? series.kind, detail: detail.join(" · ") || "No value" };
}

function create_card(example, index) {
  const card = document.createElement("article");
  card.className = "general-chart-card";
  card.dataset.category = example.category;
  card.dataset.chartIndex = String(index);
  card.innerHTML = `
    <div class="general-card-header">
      <div class="general-card-copy"><h2>${example.title}</h2><p>${example.description}</p></div>
      <span class="general-kind-pill">${example.label}</span>
    </div>
    <div class="general-chart-host" aria-label="${example.title} chart"><div class="general-chart-tooltip"></div></div>
    <div class="general-card-footer"><div class="general-card-legends"></div><span class="general-spacer"></span><span class="general-ready-state">loading</span></div>`;
  const legends = card.querySelector(".general-card-legends");
  for (const spec of example.series) {
    const chip = document.createElement("span");
    chip.className = "general-legend-chip";
    chip.style.color = spec.color ?? palette.blue;
    chip.innerHTML = `<span class="general-legend-dot"></span><span style="color:var(--text-secondary)">${spec.title}</span>`;
    legends.appendChild(chip);
  }
  return card;
}

async function mount_example(example, card) {
  const host = card.querySelector(".general-chart-host");
  const tooltip = card.querySelector(".general-chart-tooltip");
  const ready_state = card.querySelector(".general-ready-state");
  const chart = await create_chart(host, {
    autoSize: true,
    // Auto tries WebGPU first. Unsupported browsers and machines without an available adapter
    // must still render the same chart through the engine's Canvas2D fallback.
    backend: requested_backend,
    theme: active_theme,
    initialPane: { horizontal_domain: example.horizontal_domain },
    grid: { vertLines: { visible: false }, horzLines: { visible: false } },
    // This page is a vertically scrolling dashboard. Wheel/pinch gestures belong to the page,
    // otherwise every chart card traps the user's scroll and makes the workspace feel frozen.
    handle_scroll: {
      pressed_mouse_move: false,
      mouse_wheel: false,
      horz_touch_drag: false,
      vert_touch_drag: false,
    },
    handle_scale: {
      mouse_wheel: false,
      pinch: false,
      axis_double_click_reset: false,
      axis_pressed_mouse_move: false,
    },
    kinetic_scroll: false,
  });
  const pane = chart.panes()[0];
  const pane_index = pane.pane_index();
  for (const axis of example.axes) chart.add_axis({ ...axis, pane: pane_index });

  const series_by_id = new Map();
  const series_handles = [];
  for (const spec of example.series) {
    const handle = chart.add_series(spec.kind, {
      pane: pane_index,
      x_axis_id: "x",
      y_axis_id: "y",
      title: spec.title,
      color: spec.color,
      point_radius: spec.point_radius,
      data_labels: spec.data_labels,
      group_id: spec.group_id,
      stack_id: spec.stack_id,
      stack_mode: spec.stack_mode,
    });
    handle.set_data(spec.data);
    series_by_id.set(handle.id, handle);
    series_handles.push(handle);
  }
  chart.render();
  const backend_status = chart.backend_status();
  ready_state.textContent = chart.backend() === "webgpu"
    ? "WebGPU"
    : backend_status.requested_backend === "auto" ? "Canvas2D fallback" : "Canvas2D";
  ready_state.title = backend_status.detail ?? "";

  let pointer_frame = 0;
  let pointer_xy = null;
  const paint_tooltip = () => {
    pointer_frame = 0;
    if (pointer_xy === null) return;
    const { x, y, width, height } = pointer_xy;
    const hit = chart.general_hit_test(pane.pane_index(), x, y);
    const hit_series = hit === null ? null : series_by_id.get(hit.series);
    const content = hit === null || hit_series === undefined ? null : tooltip_text(hit_series, hit.row);
    if (content === null) {
      tooltip.dataset.visible = "false";
      return;
    }
    tooltip.innerHTML = `<strong>${content.title}</strong><span>${content.detail}</span>`;
    tooltip.style.left = `${Math.min(Math.max(8, x + 12), Math.max(8, width - 200))}px`;
    tooltip.style.top = `${Math.min(Math.max(8, y + 12), Math.max(8, height - 64))}px`;
    tooltip.dataset.visible = "true";
  };
  const on_pointer_move = (event) => {
    const rect = host.getBoundingClientRect();
    pointer_xy = {
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
      width: rect.width,
      height: rect.height,
    };
    if (pointer_frame === 0) pointer_frame = requestAnimationFrame(paint_tooltip);
  };
  const on_pointer_leave = () => {
    pointer_xy = null;
    tooltip.dataset.visible = "false";
  };
  host.addEventListener("pointermove", on_pointer_move);
  host.addEventListener("pointerleave", on_pointer_leave);
  return {
    chart,
    pane,
    series: series_handles,
    card,
    example,
    dispose() {
      host.removeEventListener("pointermove", on_pointer_move);
      host.removeEventListener("pointerleave", on_pointer_leave);
      if (pointer_frame !== 0) cancelAnimationFrame(pointer_frame);
      tooltip.dataset.visible = "false";
      chart.remove();
      ready_state.textContent = "paused";
    },
  };
}

let errors = [];
let states = [];
let observer = null;
let dashboard_active = false;
let visibility_clock = 0;

function update_runtime_status() {
  const active = states.flatMap((state) => state.entry === null ? [] : [state.entry]);
  metric_ready.textContent = String(active.length);
  const backend_counts = new Map();
  for (const entry of active) backend_counts.set(entry.chart.backend(), (backend_counts.get(entry.chart.backend()) ?? 0) + 1);
  const fallback = active
    .map((entry) => entry.chart.backend_status())
    .find((status) => status.requested_backend === "auto" && status.active_backend === "canvas2d");
  const summary = [...backend_counts.entries()]
    .map(([backend, count]) => `${count} × ${backend === "webgpu" ? "WebGPU" : "Canvas2D"}`)
    .join(" · ");
  runtime_badge.title = fallback?.detail ?? "";
  runtime_badge.textContent = summary === ""
    ? `${requested_backend === "auto" ? "WebGPU preferred" : "Canvas2D requested"} · waiting for viewport`
    : `${summary} live${fallback ? ` · WebGPU fallback (${fallback.reason})` : ""} · max ${MAX_ACTIVE_CHARTS}`;
}

function update_error_status() {
  if (errors.length === 0) {
    dashboard_error.dataset.visible = "false";
    dashboard_error.textContent = "";
    return;
  }
  dashboard_error.textContent = `${errors.length} chart${errors.length === 1 ? "" : "s"} failed to initialize: ${errors.map((error) => error.title).join(", ")}`;
  dashboard_error.dataset.visible = "true";
}

async function mount_state(state) {
  if (!dashboard_active || !state.desired || state.entry !== null || state.mounting !== null || state.failed || state.card.hidden) return;
  state.mounting = mount_example(state.example, state.card);
  try {
    const entry = await state.mounting;
    if (!dashboard_active || !state.desired || state.card.hidden) {
      entry.dispose();
      state.card.dataset.mounted = "false";
    } else {
      state.entry = entry;
      state.card.dataset.mounted = "true";
    }
  } catch (error) {
    console.error(`General dashboard failed to mount ${state.example.title}`, error);
    state.failed = true;
    state.card.dataset.mounted = "failed";
    state.card.querySelector(".general-ready-state").textContent = "failed";
    errors.push({ title: state.example.title, message: error?.message ?? String(error) });
    update_error_status();
  } finally {
    state.mounting = null;
    update_runtime_status();
  }
}

function reconcile_active_charts() {
  if (!dashboard_active) return;
  const selected = new Set(
    states
      .filter((state) => state.visible && !state.card.hidden && !state.failed)
      .sort((left, right) => left.distance - right.distance || left.index - right.index)
      .slice(0, MAX_ACTIVE_CHARTS),
  );
  for (const state of states) {
    state.desired = selected.has(state);
    if (state.desired) void mount_state(state);
    else unmount_state(state);
  }
}

function unmount_state(state) {
  if (state.entry === null) return;
  state.entry.dispose();
  state.entry = null;
  state.card.dataset.mounted = "false";
  update_runtime_status();
}

function ensure_observer() {
  if (observer !== null) return;
  observer = new IntersectionObserver((entries) => {
    for (const observed of entries) {
      const state = states[Number(observed.target.dataset.chartIndex)];
      if (state === undefined) continue;
      state.visible = observed.isIntersecting;
      state.last_visible = observed.isIntersecting ? ++visibility_clock : state.last_visible;
      const root_center = observed.rootBounds === null
        ? window.innerHeight / 2
        : (observed.rootBounds.top + observed.rootBounds.bottom) / 2;
      state.distance = Math.abs(
        (observed.boundingClientRect.top + observed.boundingClientRect.bottom) / 2 - root_center,
      );
    }
    reconcile_active_charts();
  }, { root: dashboard_root, rootMargin: "32px 0px", threshold: 0.15 });
}

function set_active(active) {
  dashboard_active = active;
  ensure_observer();
  if (active) {
    for (const state of states) observer.observe(state.card);
  } else {
    observer.disconnect();
    for (const state of states) {
      state.desired = false;
      state.visible = false;
      unmount_state(state);
    }
  }
  update_runtime_status();
}

function set_theme(theme) {
  active_theme = theme;
  for (const state of states) state.entry?.chart.apply_options({ theme });
}

export function install_general_dashboard({ root, backend = "auto", theme = "dark", active = false }) {
  if (!(root instanceof HTMLElement)) throw new Error("general dashboard requires a workspace root");
  if (window.__generalDashboard?.dispose instanceof Function) window.__generalDashboard.dispose();

  dashboard_root = root;
  chart_grid = root.querySelector("#general_chart_grid");
  metric_ready = root.querySelector("#general_metric_ready");
  runtime_badge = root.querySelector("#general_runtime_badge");
  dashboard_error = root.querySelector("#general_dashboard_error");
  requested_backend = backend === "canvas2d" ? "canvas2d" : "auto";
  active_theme = theme;
  errors = [];
  states = [];
  observer = null;
  dashboard_active = false;
  visibility_clock = 0;
  chart_grid.replaceChildren();

  states = examples.map((example, index) => {
    const card = create_card(example, index);
    card.dataset.mounted = "false";
    chart_grid.appendChild(card);
    return {
      example,
      card,
      entry: null,
      mounting: null,
      failed: false,
      visible: false,
      last_visible: 0,
      distance: Number.POSITIVE_INFINITY,
      desired: false,
      index,
    };
  });

  for (const button of root.querySelectorAll("[data-general-filter]")) {
    button.addEventListener("click", () => {
      const filter = button.dataset.generalFilter;
      for (const peer of root.querySelectorAll("[data-general-filter]")) {
        peer.setAttribute("aria-pressed", String(peer === button));
      }
      for (const state of states) {
        state.card.hidden = filter !== "all" && state.card.dataset.category !== filter;
        if (state.card.hidden) state.visible = false;
      }
      reconcile_active_charts();
    });
  }

  const controller = {
    ready: true,
    errors,
    summary: examples.map((example) => ({
      title: example.title,
      label: example.label,
      category: example.category,
      series_kinds: example.series.map((series) => series.kind),
    })),
    active_summary() {
      return states.flatMap((state) => state.entry === null ? [] : [{
        title: state.example.title,
        backend: state.entry.chart.backend(),
        backend_status: state.entry.chart.backend_status(),
        pane_count: state.entry.chart.panes().length,
        pane_index: state.entry.pane.pane_index(),
        series_kinds: state.entry.series.map((handle) => handle.kind),
      }]);
    },
    // Test handle: the mounted cards' live chart, pane, and series handles.
    active_entries() {
      return states.flatMap((state) => state.entry === null ? [] : [state.entry]);
    },
    set_active,
    set_theme,
    dispose() {
      set_active(false);
      observer?.disconnect();
      observer = null;
      chart_grid.replaceChildren();
    },
  };
  window.__generalDashboard = controller;
  set_active(active);
  window.dispatchEvent(new CustomEvent("general-dashboard-ready"));
  return controller;
}
