import { test, expect } from "@playwright/test";

// The lane fixture is gated behind `?feature=timeline_marks` (fixture_features.js
// `timeline_mark_fixture`): hourly bars; a same-bar pair at bar 760 folds into one token with the
// count 2, a mark 30 minutes into bar 763 lands on bar 763 (inside its span) and folds with the
// pair once bars are narrower than ~7 px, and a future mark four hours and a minute past the last
// bar projects four slots into the right-side whitespace.

async function open_marks_demo(page) {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/?backend=canvas2d&feature=timeline_marks");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
}

/** Chart-content coordinates of the lane token for `logical`, plus the canvas offset (left/top of its bounding rect). */
async function token_probe(page, logical) {
  return page.evaluate((logical) => {
    const chart = window.__chart;
    const overlay = document.querySelector("#chart_container canvas:last-of-type").getBoundingClientRect();
    return {
      x: chart.time_scale().logical_to_coordinate(logical),
      y: chart.panes()[0].get_height() - 15,
      left: overlay.left,
      top: overlay.top,
    };
  }, logical);
}

test("marks land on the bar whose span holds them and project past the last bar", async ({ page }) => {
  await open_marks_demo(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const scale = chart.time_scale();
    const points = window.__main.data();
    const y = chart.panes()[0].get_height() - 15;
    const probe = (logical) => chart.timeline_marks().hit_at(scale.logical_to_coordinate(logical), y);
    // Wide bars: every data token stands alone.
    scale.set_visible_logical_range({ from: 740, to: 790 });
    const pair = probe(760);
    const inside_span = probe(763);
    const n = points.length;
    // Show the right-side whitespace so the projected slot is in view.
    scale.set_visible_logical_range({ from: n - 40, to: n + 10 });
    const projected = probe(n - 1 + 4);
    return { pair, inside_span, projected, bar_763: points[763].time, last: points[n - 1].time };
  });
  expect(result.inside_span).not.toBeNull();
  expect(result.inside_span.logical).toBe(763);
  expect(result.inside_span.time).toBe(result.bar_763 + 1800);
  expect(result.inside_span.mark_ids).toEqual(["n1"]);
  expect(result.pair).toEqual(expect.objectContaining({ count: 2, groups: ["earnings"], label: "Earnings", logical: 760 }));
  expect([...result.pair.mark_ids].sort()).toEqual(["e2", "e3"]);
  expect(result.projected).not.toBeNull();
  expect(result.projected.projected).toBe(true);
  expect(result.projected.mark_ids).toEqual(["f1"]);
  expect(result.projected.time).toBe(result.last + 4 * 3600 + 60);
});

test("zooming out folds near tokens into one cluster anchored on the earliest slot", async ({ page }) => {
  await open_marks_demo(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const scale = chart.time_scale();
    const points = window.__main.data();
    const y = chart.panes()[0].get_height() - 15;
    const probe = () => chart.timeline_marks().hit_at(scale.logical_to_coordinate(760), y);
    scale.set_visible_logical_range({ from: 740, to: 790 });
    const apart = probe();
    scale.set_visible_logical_range({ from: 0, to: points.length - 1 });
    const folded = probe();
    return { apart, folded, bar_spacing: scale.logical_to_coordinate(1) - scale.logical_to_coordinate(0) };
  });
  expect(result.apart).toEqual(expect.objectContaining({ count: 2, groups: ["earnings"] }));
  expect(result.bar_spacing).toBeLessThan(7);
  expect(result.folded).toEqual(expect.objectContaining({ count: 3, logical: 760, groups: ["earnings", "news"], label: "3 marks" }));
  expect([...result.folded.mark_ids].sort()).toEqual(["e2", "e3", "n1"]);
});

test("a click on a token delivers the resolved hit to the click subscription", async ({ page }) => {
  await open_marks_demo(page);
  await page.evaluate(() => {
    window.__chart.time_scale().set_visible_logical_range({ from: 650, to: 750 });
  });
  const probe = await token_probe(page, 700);
  await page.mouse.move(probe.left + probe.x, probe.top + probe.y);
  await page.mouse.down();
  await page.mouse.up();
  const clicks = await page.evaluate(() => window.__timeline_mark_clicks);
  expect(clicks).toHaveLength(1);
  expect(clicks[0]).toEqual(expect.objectContaining({ logical: 700, count: 1, mark_ids: ["e1"], title: "Q3 report", label: "Earnings" }));
  // A drag from the token delivers nothing more and does not pan.
  const before = await page.evaluate(() => window.__chart.time_scale().scroll_position());
  await page.mouse.move(probe.left + probe.x, probe.top + probe.y);
  await page.mouse.down();
  await page.mouse.move(probe.left + probe.x + 40, probe.top + probe.y, { steps: 4 });
  await page.mouse.up();
  expect(await page.evaluate(() => window.__timeline_mark_clicks.length)).toBe(1);
  expect(await page.evaluate(() => window.__chart.time_scale().scroll_position())).toBe(before);
});

test("hidden groups restore from a document imported before any mark exists", async ({ page }) => {
  await open_marks_demo(page);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = () => {
      const element = document.createElement("div");
      element.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
      document.body.append(element);
      return element;
    };
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const changed = first.timeline_marks().set_group_hidden("news", true);
    const unchanged = first.timeline_marks().set_group_hidden("news", true);
    const state = first.export_state();
    first.remove();
    first_host.remove();

    const second_host = host();
    const second = await create_chart(second_host, { backend: "canvas2d", autoSize: false });
    second.import_state(state);
    const before_marks = second.timeline_marks().hidden_groups();
    second.timeline_marks().set({ marks: [{ id: "n", time: 3600, group: "news" }] });
    const after_marks = second.timeline_marks().hidden_groups();
    const canonical = second.export_state();
    let error_code = null;
    try {
      second.timeline_marks().set({ marks: [{ id: "a", time: 1, group: "g" }, { id: "a", time: 2, group: "g" }] });
    } catch (error) {
      error_code = error.code;
    }
    second.remove();
    second_host.remove();
    return { changed, unchanged, state, before_marks, after_marks, canonical, error_code };
  });
  expect(result.changed).toBe(true);
  expect(result.unchanged).toBe(false);
  expect(result.state.hidden_mark_groups).toEqual(["news"]);
  expect(result.before_marks).toEqual(["news"]);
  expect(result.after_marks).toEqual(["news"]);
  expect(result.canonical).toEqual(result.state);
  expect(result.error_code).toBe("invalid_data");
});
