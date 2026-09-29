import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { PNG } from "pngjs";

// Inline editing of B8 family text boxes through real pointer and keyboard input: double-click,
// Enter, and F2 open the text tool's caret overlay on the engine-resolved text box, typing is live
// and multi-line, a commit is one undo step and a cancel restores the text, the accessibility
// surface edits and gets its focus back, the edit persists and syncs, and a note shows its text
// only while focused. Every layout decision is engine-owned; these specs only drive the package.

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
