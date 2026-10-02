import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { PNG } from "pngjs";

// Inline editing of drawing text through real pointer and keyboard input: double-click, Enter,
// and F2 open the caret overlay on the engine-resolved layout, typing is live (multi-line in a
// family text box, one rotated line in a run label such as a ray's or a rectangle's text), a
// commit is one undo step and a cancel restores the text, the accessibility surface edits and
// gets its focus back, the edit persists and syncs, a note shows its text only while focused, and
// placing an annotation that starts from a default text opens the editor at once. Every layout
// decision is engine-owned; these specs only drive the package.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const PINK = [233, 30, 99]; // #e91e63 — collides with no fixture pixel

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function goto_fixture(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  await settle_frames(page);
}

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

/** Device-px bounding box of `target`-colored pixels, or null when none. */
function color_extent(png, target, tol = 12) {
  let box = null;
  for (let y = 0; y < png.height; y += 1) {
    for (let x = 0; x < png.width; x += 1) {
      const o = (y * png.width + x) * 4;
      if (
        Math.abs(png.data[o] - target[0]) <= tol &&
        Math.abs(png.data[o + 1] - target[1]) <= tol &&
        Math.abs(png.data[o + 2] - target[2]) <= tol
      ) {
        box = box === null
          ? { left: x, right: x, top: y, bottom: y }
          : { left: Math.min(box.left, x), right: Math.max(box.right, x), top: Math.min(box.top, y), bottom: Math.max(box.bottom, y) };
      }
    }
  }
  return box;
}

/** Primary-blue device pixels within 7 CSS px of a point: the anchor handle discs' border. */
function blue_pixels_near(png, css_x, css_y) {
  let count = 0;
  for (let y = Math.round((css_y - 7) * PR); y <= Math.round((css_y + 7) * PR); y += 1) {
    for (let x = Math.round((css_x - 7) * PR); x <= Math.round((css_x + 7) * PR); x += 1) {
      const o = (y * png.width + x) * 4;
      if (png.data[o + 2] > 200 && png.data[o] < 90 && png.data[o + 1] < 150) count += 1;
    }
  }
  return count;
}

/** Add a drawing on the visible data at `fraction` of the view; returns its id. */
async function add_on_bar(page, kind, fraction, options) {
  return page.evaluate(({ kind, fraction, options }) => {
    const chart = window.__chart;
    const range = chart.time_scale().get_visible_logical_range();
    const logical = Math.floor(range.from + (range.to - range.from) * fraction);
    const price = window.__main.data_by_index(logical).high;
    const anchors = kind === "price_note" || kind === "callout"
      ? [{ logical, price }, { logical: logical + 6, price }]
      : [{ logical, price }];
    return chart.add_drawing(kind, anchors, { color: "#e91e63", ...options }).id;
  }, { kind, fraction, options });
}

/** The engine's editor layout of a family text box in page px. */
async function edit_layout(page, id) {
  return page.evaluate((id) => {
    const json = window.__chart.wasm.drawing_text_edit_layout_json(id);
    if (json === "") return null;
    const layout = JSON.parse(json);
    const offset = document.getElementById("chart_container").getBoundingClientRect();
    return {
      ...layout,
      x: layout.x + offset.left,
      y: layout.y + offset.top,
      center: {
        x: (layout.rect[0] + layout.rect[2]) / 2 + offset.left,
        y: (layout.rect[1] + layout.rect[3]) / 2 + offset.top,
      },
    };
  }, id);
}

async function text_of(page, id) {
  return page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id)?.options().text, id);
}

/** Add a drawing across the visible data on a rising diagonal (a third anchor for channels). */
async function add_diagonal(page, kind, options, anchors = 2) {
  return page.evaluate(({ kind, options, anchors }) => {
    const chart = window.__chart;
    const range = chart.time_scale().get_visible_logical_range();
    const at = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
    let low = Infinity;
    let high = -Infinity;
    for (let index = Math.max(0, Math.floor(range.from)); index <= Math.floor(range.to); index += 1) {
      const bar = window.__main.data_by_index(index);
      if (bar) {
        low = Math.min(low, bar.low);
        high = Math.max(high, bar.high);
      }
    }
    const span = high - low;
    const points = [
      { logical: at(0.2), price: low + span * 0.15 },
      { logical: at(0.55), price: low + span * 0.6 },
      { logical: at(0.35), price: low + span * 0.85 },
    ].slice(0, anchors);
    return chart.add_drawing(kind, points, { color: "#e91e63", ...options }).id;
  }, { kind, options, anchors });
}

const CHART_CANVAS = "#chart_container canvas:last-of-type";
const EDITOR = "#chart_container #aeris_charts-text-input";
const EDITOR_WRAP = "#chart_container #aeris_charts-text-editor";

async function focus_chart(page) {
  await page.evaluate((selector) => document.querySelector(selector).focus(), CHART_CANVAS);
}

async function deselect(page) {
  await page.evaluate(() => {
    window.__chart.wasm.set_selected_drawing(undefined);
    window.__chart.render();
  });
  await settle_frames(page);
}

/** The screen point of a data coordinate, in page px. */
async function spot(page, logical, price) {
  return page.evaluate(({ logical, price }) => {
    const offset = document.getElementById("chart_container").getBoundingClientRect();
    return {
      x: window.__chart.time_scale().logical_to_coordinate(logical) + offset.left,
      y: window.__main.price_to_coordinate(price) + offset.top,
    };
  }, { logical, price });
}

test("double-click edits a family text box in place: live multi-line typing, one undo step, Escape restores", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_on_bar(page, "comment", 0.4);
  await settle_frames(page);
  const before = await edit_layout(page, id);
  const editor = page.locator("#chart_container #aeris_charts-text-input");

  // A double-click on the box selects it and opens the caret overlay on the engine's text box.
  await page.mouse.dblclick(before.center.x, before.center.y);
  await expect(editor).toBeFocused();
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBe(id);
  // A labeled native multi-line text box (the text tool's single-line run is content-editable).
  await expect(page.getByRole("textbox", { name: "comment text" })).toBeFocused();
  expect(await editor.evaluate((el) => el.tagName)).toBe("TEXTAREA");
  expect(await editor.evaluate((el) => getComputedStyle(el).opacity)).toBe("0");
  const wrap_box = await page.locator("#chart_container #aeris_charts-text-editor").boundingBox();
  expect(Math.abs(wrap_box.x - before.x)).toBeLessThan(1);
  expect(Math.abs(wrap_box.y - (before.y - before.line_height / 2))).toBeLessThan(1);
  // The caret sits after the text on the first line, in the box's contrasting ink.
  const caret = page.locator("#aeris_charts-text-caret");
  const caret_start = await caret.boundingBox();
  expect(caret_start.x).toBeGreaterThan(before.x + 10);
  expect(await caret.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe("rgb(255, 255, 255)");

  // Typing is live; Shift+Enter adds a line, and the bottom-aligned bubble grows upward.
  const history_before = await page.evaluate(() => window.__chart.can_undo_drawing());
  await page.keyboard.type(" one");
  await expect.poll(() => text_of(page, id)).toBe("Comment one");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("two");
  await expect.poll(() => text_of(page, id)).toBe("Comment one\ntwo");
  const grown = await edit_layout(page, id);
  expect(grown.y).toBeCloseTo(before.y - before.line_height, 3);
  const caret_second = await caret.boundingBox();
  expect(caret_second.y - caret_start.y).toBeCloseTo(0, 0);
  expect(caret_second.x).toBeLessThan(caret_start.x);
  const wrap_grown = await page.locator("#chart_container #aeris_charts-text-editor").boundingBox();
  expect(Math.abs(wrap_grown.y - (grown.y - grown.line_height / 2))).toBeLessThan(1);

  // Enter commits the whole edit as one undo step.
  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(0);
  expect(history_before).toBe(true);
  expect(await text_of(page, id)).toBe("Comment one\ntwo");
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await text_of(page, id)).toBe("Comment");
  expect(await page.evaluate(() => window.__chart.redo_drawing())).toBe(true);
  expect(await text_of(page, id)).toBe("Comment one\ntwo");

  // Enter on the selected drawing reopens it; Escape restores the text without a history entry.
  await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
  await page.keyboard.press("Enter");
  await expect(editor).toBeFocused();
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Backspace");
  await expect.poll(() => text_of(page, id)).toBe("Comment one\nt");
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  expect(await text_of(page, id)).toBe("Comment one\ntwo");
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await text_of(page, id), "undo reverts the committed edit, not the cancelled one").toBe("Comment");

  // Clearing the text keeps one caret line in the box while typing, and paints no box after.
  await page.evaluate(() => window.__chart.redo_drawing());
  await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
  await page.keyboard.press("F2");
  await expect(editor).toBeFocused();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await expect.poll(() => text_of(page, id)).toBe("");
  await settle_frames(page);
  expect(color_extent(await capture(page), PINK), "the emptied box keeps its caret line").not.toBeNull();
  await page.keyboard.press("Enter");
  expect(await text_of(page, id)).toBe("");
});

test("every family text box opens the editor; locked drawings and shapes without text do not", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  const kinds = ["anchored_text", "note", "price_note", "callout", "comment", "price_label", "signpost", "arrow_mark_up"];
  for (const [index, kind] of kinds.entries()) {
    const id = kind === "anchored_text"
      ? await page.evaluate(() => window.__chart.add_drawing("anchored_text", [{ logical: 0.3, price: 0.3 }], { color: "#e91e63" }).id)
      : await add_on_bar(page, kind, 0.1 + index * 0.1);
    await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
    await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
    await page.keyboard.press("Enter");
    await expect(editor, kind).toBeFocused();
    await expect(editor).toHaveAttribute("aria-label", `${kind.replaceAll("_", " ")} text`);
    await page.keyboard.type("!");
    await page.keyboard.press("Enter");
    await expect(editor).toHaveCount(0);
    expect((await text_of(page, id)).endsWith("!"), kind).toBe(true);
    await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).remove(), id);
  }
  for (const [kind, options] of [["flag_mark", {}], ["icon", {}], ["comment", { locked: true }]]) {
    const id = await add_on_bar(page, kind, 0.5, options);
    await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
    await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
    await page.keyboard.press("Enter");
    await page.keyboard.press("F2");
    await expect(editor, kind).toHaveCount(0);
    await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).remove(), id);
  }
});

test("F2 on the accessibility drawing target edits the text and returns focus to the target", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_on_bar(page, "callout", 0.3, { text: "Callout" });
  await settle_frames(page);
  await page.evaluate((id) => window.__chart.accessibility().focus_target(`drawing:${id}`), id);
  const target = () => page.evaluate(() => document.activeElement?.dataset?.a11yTarget ?? null);
  expect(await target()).toBe(`drawing:${id}`);
  await page.keyboard.press("F2");
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeFocused();
  await settle_frames(page);
  // The accessibility layer hides the chart's other controls, never its inline editor.
  expect(await editor.getAttribute("aria-hidden")).toBeNull();
  await page.keyboard.type(" edited");
  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(0);
  expect(await text_of(page, id)).toBe("Callout edited");
  expect(await target()).toBe(`drawing:${id}`);
});

test("edited family text persists through export/import and sync", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_on_bar(page, "signpost", 0.5);
  await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
  await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
  await page.keyboard.press("Enter");
  await page.keyboard.type(" A");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("B");
  await page.keyboard.press("Enter");
  expect(await text_of(page, id)).toBe("Signpost A\nB");
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const state = window.__chart.export_state();
    const sync = window.__chart.drawing_sync_payload("cell-a");
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
    document.body.append(host);
    const restored = await create_chart(host, { backend: "canvas2d", autoSize: false });
    restored.import_state(state);
    const imported = restored.drawings().map((drawing) => drawing.options().text);
    restored.remove();
    const mirror_host = document.createElement("div");
    mirror_host.style.cssText = host.style.cssText;
    document.body.append(mirror_host);
    const mirror = await create_chart(mirror_host, { backend: "canvas2d", autoSize: false });
    const applied = mirror.apply_drawing_sync_payload(sync);
    const synced = mirror.drawings().map((drawing) => drawing.options().text);
    mirror.remove();
    host.remove();
    mirror_host.remove();
    return { persisted: state.drawings.map((drawing) => drawing.style.text), imported, applied, synced };
  });
  expect(result.persisted).toEqual(["Signpost A\nB"]);
  expect(result.imported).toEqual(["Signpost A\nB"]);
  expect(result.applied).toBe(true);
  expect(result.synced).toEqual(["Signpost A\nB"]);
});

test("a note shows its text only while hovered or selected unless it always shows it", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_on_bar(page, "note", 0.5);
  await page.evaluate(() => window.__chart.wasm.set_selected_drawing(undefined));
  await settle_frames(page);
  const pin = color_extent(await capture(page), PINK);
  expect(pin).not.toBeNull();
  expect((pin.right - pin.left) / PR, "only the pin paints at rest").toBeLessThan(20);
  const tip = await page.evaluate((id) => {
    const point = window.__chart.drawings().find((drawing) => drawing.id === id).points()[0];
    return {
      x: window.__chart.time_scale().logical_to_coordinate(point.logical),
      y: window.__main.price_to_coordinate(point.price),
    };
  }, id);

  // Hovering the pin head reveals the box beside it; leaving hides it again.
  await page.mouse.move(tip.x, tip.y - 17);
  await settle_frames(page);
  const hovered = color_extent(await capture(page), PINK);
  expect((hovered.right - hovered.left) / PR).toBeGreaterThan(30);
  await page.mouse.move(tip.x, tip.y + 150);
  await settle_frames(page);
  expect(color_extent(await capture(page), PINK)).toEqual(pin);

  // Selection keeps it; the always-visible option keeps it without focus.
  await page.mouse.click(tip.x, tip.y - 17);
  await page.mouse.move(tip.x, tip.y + 150);
  await settle_frames(page);
  expect((color_extent(await capture(page), PINK).right - pin.left) / PR).toBeGreaterThan(30);
  await page.evaluate((id) => {
    const chart = window.__chart;
    chart.wasm.set_selected_drawing(undefined);
    chart.drawings().find((drawing) => drawing.id === id).apply_options({ tool_options: { projection_annotation: { always_show_text: true } } });
  }, id);
  await settle_frames(page);
  const always = color_extent(await capture(page), PINK);
  expect((always.right - always.left) / PR).toBeGreaterThan(30);
  expect(await page.evaluate((id) => window.__chart.drawing_kind_options(id), id))
    .toMatchObject({ kind: "projection_annotation", always_show_text: true });
});

test("a double-click on a trend line's body opens its editor like every other text tool", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_diagonal(page, "trend_line", { text: "trend label" });
  await deselect(page);
  // A point on the stroke a fifth of the way along, away from the label: the engine does not
  // answer for the label there, so only the body-double-click rule can open the editor.
  const probe = await page.evaluate((id) => {
    const chart = window.__chart;
    const [a, b] = chart.drawings().find((drawing) => drawing.id === id).points();
    const at = (t) => ({
      x: chart.time_scale().logical_to_coordinate(a.logical + (b.logical - a.logical) * t),
      y: window.__main.price_to_coordinate(a.price + (b.price - a.price) * t),
    });
    const point = at(0.2);
    const offset = document.getElementById("chart_container").getBoundingClientRect();
    return {
      x: point.x + offset.left,
      y: point.y + offset.top,
      label_hit: Number(chart.wasm.drawing_text_hit_at(point.x, point.y)),
      drawing_at: Number(chart.wasm.drawing_at(point.x, point.y)),
    };
  }, id);
  expect(probe.label_hit, "the probe is on the body, not the label").toBe(0);
  expect(probe.drawing_at, "the probe is on the trend line").toBe(id);
  const editor = page.locator(EDITOR);
  await page.mouse.dblclick(probe.x, probe.y);
  await expect(editor).toBeFocused();
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBe(id);
  expect(await text_of(page, id)).toBe("trend label");
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  expect(await text_of(page, id), "Escape leaves the text as it was").toBe("trend label");
});

test("a ray's text edits in place along its stroke: one rotated line in the stroke's ink, one undo step", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_diagonal(page, "ray", { text: "ray label" });
  await deselect(page);
  const before = await edit_layout(page, id);
  expect(before.multiline).toBe(false);
  expect(Math.abs(before.angle), "the ray's label follows its slope").toBeGreaterThan(0.1);
  const editor = page.locator(EDITOR);
  const wrap = page.locator(EDITOR_WRAP);

  // A double-click on the label (its center, off the stroke in the default top-right slot)
  // selects the ray and opens the editor: one content-editable line, labeled by the tool.
  await page.mouse.dblclick(before.center.x, before.center.y);
  await expect(editor).toBeFocused();
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBe(id);
  await expect(page.getByRole("textbox", { name: "ray text" })).toBeFocused();
  expect(await editor.evaluate((el) => [el.tagName, el.isContentEditable])).toEqual(["DIV", true]);
  expect(await editor.evaluate((el) => getComputedStyle(el).opacity)).toBe("0");

  // The wrap's left-middle sits on the engine's run start and rotates about it by the layout's
  // angle; the caret paints in the stroke's ink.
  const offset = await page.evaluate(() => {
    const rect = document.getElementById("chart_container").getBoundingClientRect();
    return { left: rect.left, top: rect.top };
  });
  const placed = await wrap.evaluate((el) => {
    const matrix = new DOMMatrixReadOnly(getComputedStyle(el).transform);
    return {
      left: Number.parseFloat(el.style.left),
      angle: Math.atan2(matrix.b, matrix.a),
      origin_x: el.style.transformOrigin.split(" ")[0],
    };
  });
  expect(placed.left).toBeCloseTo(before.x - offset.left, 1);
  expect(placed.angle).toBeCloseTo(before.angle, 3);
  expect(placed.origin_x).toBe("0px");
  expect(await page.locator("#aeris_charts-text-caret").evaluate((el) => getComputedStyle(el).backgroundColor))
    .toBe("rgb(233, 30, 99)");
  expect(before.color, "the layout's ink is the stroke color").toBe("#e91e63");

  // Typing is live: the text repaints and the right-aligned run grows leftwards along the stroke.
  const history_before = await page.evaluate(() => window.__chart.can_undo_drawing());
  await page.keyboard.type(" more");
  await expect.poll(() => text_of(page, id)).toBe("ray label more");
  const grown = await edit_layout(page, id);
  expect(grown.x).toBeLessThan(before.x);
  expect(grown.rect[2] - grown.rect[0]).toBeGreaterThan(before.rect[2] - before.rect[0]);
  expect(grown.angle).toBeCloseTo(before.angle, 6);

  // Enter commits the whole edit as one undo step.
  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(0);
  expect(history_before).toBe(true);
  expect(await text_of(page, id)).toBe("ray label more");
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await text_of(page, id)).toBe("ray label");
  expect(await page.evaluate(() => window.__chart.redo_drawing())).toBe(true);

  // Escape restores the text without a history entry.
  await focus_chart(page);
  await page.keyboard.press("F2");
  await expect(editor).toBeFocused();
  await page.keyboard.type(" discarded");
  await expect.poll(() => text_of(page, id)).toBe("ray label more discarded");
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  expect(await text_of(page, id)).toBe("ray label more");
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await text_of(page, id), "undo reverts the committed edit, not the cancelled one").toBe("ray label");
});

test("double-clicking the text inside an unselected rectangle or ellipse selects it and opens the editor", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  for (const kind of ["rectangle", "ellipse"]) {
    const id = await add_diagonal(page, kind, { text: `${kind} text` });
    await deselect(page);
    const layout = await edit_layout(page, id);
    expect(layout.multiline, kind).toBe(false);

    // The interior of an unselected shape is a pan surface; only its text reaches the drawing.
    // One click selects it (the text tool's two-step rule) and opens nothing.
    await page.mouse.click(layout.center.x, layout.center.y);
    expect(await page.evaluate(() => window.__chart.selected_drawing()?.id), kind).toBe(id);
    await expect(editor).toHaveCount(0);
    await page.waitForTimeout(700);

    // Click 1 of a double-click selects the unselected shape, click 2 opens the editor.
    await deselect(page);
    await page.mouse.dblclick(layout.center.x, layout.center.y);
    await expect(editor, kind).toBeFocused();
    await expect(editor).toHaveAttribute("aria-label", `${kind} text`);
    expect(await page.evaluate(() => window.__chart.selected_drawing()?.id), kind).toBe(id);
    expect(await page.evaluate(() => window.__chart.wasm.editing_drawing()), kind).toBe(id);
    expect(await editor.evaluate((el) => [el.tagName, el.isContentEditable]), kind).toEqual(["DIV", true]);
    await page.keyboard.type("!");
    await page.keyboard.press("Enter");
    await expect(editor).toHaveCount(0);
    expect(await text_of(page, id), kind).toBe(`${kind} text!`);
    await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).remove(), id);
    await page.waitForTimeout(700);
  }
});

test("an unselected drawing's label wins the hover and the click only where nothing paints above it", async ({ page }) => {
  await goto_fixture(page);
  const id = await add_diagonal(page, "rectangle", { text: "under" });
  await deselect(page);
  const layout = await edit_layout(page, id);
  const cursor = () => page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });

  // Over the label: the text cursor. Elsewhere in the interior: no drawing at all.
  await page.mouse.move(layout.center.x, layout.center.y);
  await settle_frames(page);
  expect(await cursor()).toBe("text");
  await page.mouse.move(layout.center.x, layout.center.y + 60);
  await settle_frames(page);
  expect(await cursor()).not.toBe("text");

  // A line drawn through the label covers it: the higher drawing's body wins the point.
  const label_hit = (point) => page.evaluate(({ x, y }) => {
    const offset = document.getElementById("chart_container").getBoundingClientRect();
    return window.__chart.wasm.drawing_text_hit_at(x - offset.left, y - offset.top);
  }, point);
  expect(await label_hit(layout.center)).toBe(id);
  const cover = await page.evaluate((id) => {
    const chart = window.__chart;
    const points = chart.drawings().find((drawing) => drawing.id === id).points();
    const middle = { logical: (points[0].logical + points[1].logical) / 2, price: (points[0].price + points[1].price) / 2 };
    return chart.add_drawing("horizontal_line", [{ logical: middle.logical, price: middle.price }], { color: "#e91e63" }).id;
  }, id);
  await deselect(page);
  expect(await label_hit(layout.center)).toBe(0);
  await page.evaluate((cover) => window.__chart.drawings().find((drawing) => drawing.id === cover).remove(), cover);
  await deselect(page);
  expect(await label_hit(layout.center)).toBe(id);
});

test("F2 on the accessibility target of a channel or a Fibonacci tool edits its text and returns focus", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  for (const [kind, anchors] of [["parallel_channel", 3], ["fib_retracement", 2]]) {
    const id = await add_diagonal(page, kind, { text: kind }, anchors);
    await settle_frames(page);
    await page.evaluate((id) => window.__chart.accessibility().focus_target(`drawing:${id}`), id);
    const target = () => page.evaluate(() => document.activeElement?.dataset?.a11yTarget ?? null);
    expect(await target(), kind).toBe(`drawing:${id}`);
    await page.keyboard.press("F2");
    await expect(editor, kind).toBeFocused();
    await expect(editor).toHaveAttribute("aria-label", `${kind.replaceAll("_", " ")} text`);
    await page.keyboard.type("!");
    await page.keyboard.press("Enter");
    await expect(editor).toHaveCount(0);
    expect(await text_of(page, id), kind).toBe(`${kind}!`);
    expect(await target(), kind).toBe(`drawing:${id}`);
    await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).remove(), id);
  }
});

test("Enter or F2 opens an empty run label; leaving it empty keeps every drawing", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  const id = await add_diagonal(page, "rectangle", {});
  await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
  await focus_chart(page);
  await page.keyboard.press("Enter");
  await expect(editor).toBeFocused();
  const opened = await edit_layout(page, id);
  expect(opened.multiline).toBe(false);
  await page.keyboard.type("first label");
  await expect.poll(() => text_of(page, id)).toBe("first label");
  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(0);

  // Clearing it and committing keeps the drawing (only the text tool removes itself).
  await focus_chart(page);
  await page.keyboard.press("F2");
  await expect(editor).toBeFocused();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(0);
  expect(await text_of(page, id)).toBe("");
  expect(await page.evaluate(() => window.__chart.drawings().length)).toBe(1);

  // A one-line editor keeps the text on one line: the engine owns the rule.
  await focus_chart(page);
  await page.keyboard.press("Enter");
  await expect(editor).toBeFocused();
  await page.evaluate((text) => {
    const input = document.getElementById("aeris_charts-text-input");
    input.textContent = text;
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  }, "one\ntwo");
  await expect.poll(() => text_of(page, id)).toBe("one two");
  await page.keyboard.press("Escape");
  expect(await text_of(page, id)).toBe("");
});

test("Enter or F2 on a selected drawing scrolled out of view opens no invisible editor", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  const id = await add_diagonal(page, "rectangle", { text: "off screen" });
  await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
  await settle_frames(page);
  const home = await page.evaluate(() => window.__chart.time_scale().get_visible_logical_range());
  await page.evaluate(() => {
    const time = window.__chart.time_scale();
    const range = time.get_visible_logical_range();
    time.set_visible_logical_range({ from: range.from - 1500, to: range.to - 1500 });
  });
  await settle_frames(page);
  const away = await edit_layout(page, id);
  expect(away.center.x, "the drawing's text is outside the chart").toBeGreaterThan(1280);
  await focus_chart(page);
  await page.keyboard.press("Enter");
  await page.keyboard.press("F2");
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();

  // Back in view, the same key opens it.
  await page.evaluate((home) => window.__chart.time_scale().set_visible_logical_range(home), home);
  await settle_frames(page);
  await focus_chart(page);
  await page.keyboard.press("Enter");
  await expect(editor).toBeFocused();
});

/** Place `kind` with the armed tool: click each anchor spot in turn. */
async function place_with_tool(page, kind, tap) {
  const range = await page.evaluate(() => window.__chart.time_scale().get_visible_logical_range());
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.4);
  const b0 = await page.evaluate((logical) => window.__main.data_by_index(logical), l0);
  const b1 = await page.evaluate((logical) => window.__main.data_by_index(logical), l0 + 4);
  const spots = kind === "callout" || kind === "price_note"
    ? [[l0, b0.low], [l0 + 4, b1.high]]
    : [[l0, b0.high]];
  await page.evaluate((kind) => {
    window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
    if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
  }, kind);
  for (const [logical, price] of spots) {
    const point = await spot(page, logical, price);
    await tap(point.x, point.y);
    // Taps inside the recognizer's double-tap window are one gesture, not two placements.
    await page.waitForTimeout(650);
  }
  await settle_frames(page);
}

for (const touch of [false, true]) {
  test.describe(touch ? "touch placement" : "mouse placement", () => {
    test.use({ hasTouch: touch });
    const tap = (page) => (x, y) => (touch ? page.touchscreen.tap(x, y) : page.mouse.click(x, y));

    test("placing a callout opens its editor on the default text; commit keeps the drawing and its handles", async ({ page }) => {
      await goto_fixture(page);
      const editor = page.locator(EDITOR);
      await place_with_tool(page, "callout", tap(page));
      const id = await page.evaluate(() => window.__chart.drawings().at(-1)?.id);
      expect(id).toBeGreaterThan(0);
      await expect(editor).toBeFocused();
      await expect(editor).toHaveAttribute("aria-label", "callout text");
      expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBe(id);
      expect(await text_of(page, id)).toBe("Callout");
      expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();

      // The caret sits after the default text: typing extends it, Enter commits one undo step.
      await page.keyboard.type(" one");
      await expect.poll(() => text_of(page, id)).toBe("Callout one");
      await page.keyboard.press("Enter");
      await expect(editor).toHaveCount(0);
      expect(await text_of(page, id)).toBe("Callout one");
      expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
      expect(await text_of(page, id), "undo returns the default text").toBe("Callout");
      expect(await page.evaluate(() => window.__chart.drawings().length), "the drawing remains").toBe(1);
      expect(await page.evaluate(() => window.__chart.redo_drawing())).toBe(true);

      // The placed callout keeps its two anchor handles: the selection paints a primary-blue
      // disc on each anchor (the text tool's focus border is not painted for it).
      await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
      await settle_frames(page);
      const anchors = await page.evaluate((id) => {
        const offset = document.getElementById("chart_container").getBoundingClientRect();
        return [0, 1].map((index) => {
          const [x, y] = window.__chart.wasm.drawing_point_to_coordinate(id, index);
          return { x: x + offset.left, y: y + offset.top };
        });
      }, id);
      expect(anchors).toHaveLength(2);
      const png = await capture(page);
      for (const [index, anchor] of anchors.entries()) {
        expect(blue_pixels_near(png, anchor.x, anchor.y), `anchor ${index} handle`).toBeGreaterThan(4);
      }
    });

    test("placing a note opens its editor; Escape keeps the default text and the drawing", async ({ page }) => {
      await goto_fixture(page);
      const editor = page.locator(EDITOR);
      await place_with_tool(page, "note", tap(page));
      const id = await page.evaluate(() => window.__chart.drawings().at(-1)?.id);
      await expect(editor).toBeFocused();
      await expect(editor).toHaveAttribute("aria-label", "note text");
      await page.keyboard.type(" discarded");
      await expect.poll(() => text_of(page, id)).toBe("Note discarded");
      await page.keyboard.press("Escape");
      await expect(editor).toHaveCount(0);
      expect(await text_of(page, id)).toBe("Note");
      expect(await page.evaluate(() => window.__chart.drawings().length)).toBe(1);
      // Escape recorded no history: the only undo step is the placement itself.
      expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
      expect(await page.evaluate(() => window.__chart.drawings().length)).toBe(0);
    });

    test("placing an arrow mark, a price label, or a price note opens no editor", async ({ page }) => {
      await goto_fixture(page);
      for (const kind of ["arrow_mark_up", "price_label", "price_note"]) {
        await place_with_tool(page, kind, tap(page));
        await expect(page.locator(EDITOR), kind).toHaveCount(0);
        expect(await page.evaluate(() => window.__chart.wasm.editing_drawing()), kind).toBeUndefined();
      }
      expect(await page.evaluate(() => window.__chart.drawings().length)).toBe(3);
    });
  });
}

test("a host dbl_click handler can take focus for its own panel: the editor closes untouched and the panel keeps focus", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  const id = await add_diagonal(page, "rectangle", { text: "settings" });
  await deselect(page);
  const layout = await edit_layout(page, id);
  await page.evaluate(() => {
    const input = document.createElement("input");
    input.id = "host_panel_input";
    input.style.cssText = "position:fixed;left:8px;bottom:8px;z-index:10";
    document.body.append(input);
    window.__host_focus_events = [];
    window.__chart.subscribe_dbl_click(() => input.focus());
    for (const type of ["focusin", "focusout"]) {
      document.addEventListener(type, (event) => window.__host_focus_events.push(`${type}:${event.target.id || event.target.tagName}`));
    }
  });

  // The double-click opens the editor, then the host's handler moves focus to its own input.
  await page.mouse.dblclick(layout.center.x, layout.center.y);
  await expect(page.locator("#host_panel_input")).toBeFocused();
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();
  // Nothing steals focus back afterwards: not at once, and not after the browser settles.
  await page.waitForTimeout(100);
  expect(await page.evaluate(() => document.activeElement?.id)).toBe("host_panel_input");
  expect(await page.evaluate(() => window.__host_focus_events.at(-1))).toBe("focusin:host_panel_input");

  // The drawing is exactly as it was: the same text, and no undo step beyond its own placement.
  expect(await text_of(page, id)).toBe("settings");
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await page.evaluate(() => window.__chart.drawings().length)).toBe(0);
});

test("clicking a host control while an editor is open commits it and leaves focus on that control", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  const id = await add_diagonal(page, "ray", { text: "label" }, 2);
  await page.evaluate((id) => { window.__chart.wasm.set_selected_drawing(id); window.__chart.render(); }, id);
  await focus_chart(page);
  await page.keyboard.press("Enter");
  await expect(editor).toBeFocused();
  await page.keyboard.type("!");
  await page.evaluate(() => {
    const input = document.createElement("input");
    input.id = "host_panel_input";
    input.style.cssText = "position:fixed;left:8px;bottom:8px;z-index:10";
    document.body.append(input);
  });
  await page.locator("#host_panel_input").click();
  await expect(editor).toHaveCount(0);
  await expect(page.locator("#host_panel_input")).toBeFocused();
  expect(await text_of(page, id)).toBe("label!");

  // Escape and Enter still return focus to the chart, which is where the editor opened from.
  await focus_chart(page);
  await page.keyboard.press("F2");
  await expect(editor).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement === document.querySelector("#chart_container canvas:last-of-type"))).toBe(true);
});

/** The demo order's cancel control and the drawing spot the double-click specs aim at. */
async function open_trading_demo(page) {
  await page.goto("/?feature=trading&backend=canvas2d");
  await page.waitForFunction(() => window.__demo_catalogs?.lab.active_ids().includes("trading-bracket"));
  await settle_frames(page);
}

/** A rectangle with text across the visible range, its price band from chart y `top` down `height` px. */
async function add_band(page, top, height, text) {
  return page.evaluate(({ top, height, text }) => {
    const chart = window.__chart;
    const range = chart.time_scale().get_visible_logical_range();
    return chart.add_drawing("rectangle", [
      { logical: range.from + 1, price: window.__main.coordinate_to_price(top) },
      { logical: range.to - 1, price: window.__main.coordinate_to_price(top + height) },
    ], { color: "#e91e63", text }).id;
  }, { top, height, text });
}

/** Select the drawing by clicking its text twice, apart from a double-click window. */
async function select_by_text(page, id) {
  await deselect(page);
  const layout = await edit_layout(page, id);
  await page.mouse.click(layout.center.x, layout.center.y);
  await page.waitForTimeout(700);
  await page.mouse.click(layout.center.x, layout.center.y);
  await page.waitForTimeout(700);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
}

// A band well above the order, and one that covers the order's cancel control itself.
for (const [where, band] of [["far from the drawing", { above: 200, height: 60 }], ["under the drawing", { above: 30, height: 90 }]]) {
  test(`a double-click on a trading cancel control ${where} never opens a selected drawing's editor`, async ({ page }) => {
    await open_trading_demo(page);
    const editor = page.locator(EDITOR);
    const probe = await page.evaluate(() => {
      window.__cancel_intents = [];
      window.__chart.trading().subscribe_intents((intent) => window.__cancel_intents.push(intent));
      const order = window.__chart.trading().state().orders.find((item) => item.id === "demo-stop");
      const canvas = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
      const y = window.__main.price_to_coordinate(order.price);
      const width = Math.round(window.__chart.time_scale().width());
      let first = null;
      let last = null;
      for (let x = width; x > width - 320; x -= 1) {
        const hit = window.__chart.trading().hit_at(x, y);
        if (hit?.id === order.id && hit.kind === "cancel_button") {
          if (last === null) last = x;
          first = x;
        } else if (last !== null) break;
      }
      return { x: canvas.left + (first + last) / 2, y: canvas.top + y, chart_y: y };
    });
    const id = await add_band(page, probe.chart_y - band.above, band.height, "guarded");
    await settle_frames(page);
    await select_by_text(page, id);

    await page.mouse.dblclick(probe.x, probe.y);
    expect(await page.evaluate(() => window.__cancel_intents.length), "the control still cancels").toBeGreaterThan(0);
    await expect(editor).toHaveCount(0);
    expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();
    expect(await text_of(page, id)).toBe("guarded");
  });
}

test.describe("touch", () => {
  test.use({ hasTouch: true });
  test("a double-tap on a trading cancel control under a selected drawing never opens its editor", async ({ page }) => {
    await open_trading_demo(page);
    const editor = page.locator(EDITOR);
    const probe = await page.evaluate(() => {
      window.__cancel_intents = [];
      window.__chart.trading().subscribe_intents((intent) => window.__cancel_intents.push(intent));
      const order = window.__chart.trading().state().orders.find((item) => item.id === "demo-stop");
      const canvas = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
      const y = window.__main.price_to_coordinate(order.price);
      const width = Math.round(window.__chart.time_scale().width());
      let first = null;
      let last = null;
      for (let x = width; x > width - 320; x -= 1) {
        const hit = window.__chart.trading().hit_at(x, y);
        if (hit?.id === order.id && hit.kind === "cancel_button") {
          if (last === null) last = x;
          first = x;
        } else if (last !== null) break;
      }
      return { x: canvas.left + (first + last) / 2, y: canvas.top + y, chart_y: y };
    });
    const id = await add_band(page, probe.chart_y - 30, 90, "guarded");
    await settle_frames(page);
    await select_by_text(page, id);
    await page.touchscreen.tap(probe.x, probe.y);
    await page.touchscreen.tap(probe.x, probe.y);
    await settle_frames(page);
    expect(await page.evaluate(() => window.__cancel_intents.length), "the control still cancels").toBeGreaterThan(0);
    await expect(editor).toHaveCount(0);
    expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();
  });
});

test("a double-click on the alert chip never opens a selected drawing's editor", async ({ page }) => {
  await open_trading_demo(page);
  const editor = page.locator(EDITOR);
  // The chip is hidden over a drawing's body, so the band sits well above the price it aims at.
  const y = await page.evaluate(() => window.__main.price_to_coordinate(104));
  const id = await add_band(page, y - 230, 60, "alert band");
  await settle_frames(page);
  await select_by_text(page, id);
  const chip = await page.evaluate((y) => {
    const canvas = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    return { x: canvas.left + window.__chart.time_scale().width() - 11.5, y: canvas.top + y };
  }, y);
  await page.mouse.move(chip.x, chip.y);
  await settle_frames(page);
  expect(await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").style.cursor)).toBe("pointer");
  await page.mouse.dblclick(chip.x, chip.y);
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();
});

test("the double-click activation acts only on the drawing under the pointer", async ({ page }) => {
  await goto_fixture(page);
  const editor = page.locator(EDITOR);
  const id = await add_diagonal(page, "rectangle", { text: "here" });
  await select_by_text(page, id);
  const layout = await edit_layout(page, id);
  const away = await page.evaluate(() => {
    const canvas = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    return { x: canvas.left + 12, y: canvas.top + 12 };
  });
  // The gesture layer hands the point over without a click of its own; the drawing is selected
  // (and was at press), but the point is elsewhere.
  await page.evaluate(({ x, y }) => {
    const offset = document.getElementById("chart_container").getBoundingClientRect();
    window.__chart.note_drawing_press();
    window.__chart.activate_drawing_double_click(x - offset.left, y - offset.top);
  }, away);
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__chart.wasm.editing_drawing())).toBeUndefined();
  await page.evaluate(({ x, y }) => {
    const offset = document.getElementById("chart_container").getBoundingClientRect();
    window.__chart.activate_drawing_double_click(x - offset.left, y - offset.top);
  }, layout.center);
  await expect(editor).toBeFocused();
});
