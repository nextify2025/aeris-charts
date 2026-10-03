import { test, expect } from "@playwright/test";

// Browser gesture cancellation (`gestures.ts` `cancel_active_input`). Six representative real
// mouse gestures — pane pan, price-axis scale, drawing body drag, trading line drag, brush
// capture, and pane separator resize — are interrupted mid-drag by each cancellation source the
// recognizer listens for: window blur, the document turning hidden, lost pointer capture,
// pointercancel, a host resize (ResizeObserver), and the package's backend-lost event. A
// cancelled gesture stops following the still-held button, commits nothing that only a release
// would commit (drawing points roll back, no broker intent, no brush stroke), and leaves the
// recognizer ready for the next real gesture.
//
// Every case crosses the 5 px click slop before it cancels, so its release is a drag release.
// Whether a cancel before the slop should also swallow the release's click, and whether a
// cancelled pan, axis scale, or separator resize keeps or restores its partial offset, are open
// product decisions; those cases assert only that the gesture stopped.

const OVERLAY = "#chart_container canvas:last-of-type";

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function goto_fixture(page, feature = null) {
  const query = feature === null ? "" : `&feature=${feature}`;
  await page.goto(`/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1${query}`);
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate((selector) => {
    // The runtime fixture sizes the chart manually; auto-size lets a real viewport resize reach
    // the recognizer's ResizeObserver the way it does in an embedding host.
    window.__chart.apply_options({ autoSize: true });
    window.__clicks = 0;
    window.__chart.subscribe_click(() => { window.__clicks += 1; });
    window.__lost_captures = 0;
    const overlay = document.querySelector(selector);
    overlay.addEventListener("pointerdown", (event) => { window.__pointer_id = event.pointerId; }, true);
    overlay.addEventListener("lostpointercapture", () => { window.__lost_captures += 1; });
  }, OVERLAY);
  await settle_frames(page);
}

/** Client-space pane geometry: pane x = 0 sits at `left`, chart y = 0 at `top`. */
async function geometry(page) {
  return page.evaluate((selector) => {
    const rect = document.querySelector(selector).getBoundingClientRect();
    const wasm = window.__chart.wasm;
    return {
      left: rect.left + wasm.pane_left(),
      top: rect.top,
      width: wasm.time_scale_width(),
      pane_height: wasm.pane_height(0),
      overlay_width: rect.width,
    };
  }, OVERLAY);
}

async function move_through(page, points) {
  for (const point of points) await page.mouse.move(point.x, point.y);
}

/** `count` evenly spaced points from `from` (exclusive) to `to` (inclusive). */
function line(from, to, count) {
  return Array.from({ length: count }, (_, index) => {
    const t = (index + 1) / count;
    return { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t };
  });
}

async function drag(page, from, to, steps = 8) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await move_through(page, line(from, to, steps));
  await page.mouse.up();
}

const gestures = [
  {
    name: "pane pan",
    async prepare(page) {
      await goto_fixture(page);
      const g = await geometry(page);
      const from = { x: g.left + g.width * 0.6, y: g.top + g.pane_height * 0.3 };
      return {
        from,
        before: line(from, { x: from.x - 60, y: from.y }, 6),
        after: line({ x: from.x - 60, y: from.y }, { x: from.x - 160, y: from.y }, 5),
      };
    },
    read: (page) => page.evaluate(() => ({ offset: window.__chart.wasm.scroll_position() })),
    started(start, mid) {
      expect(mid.offset).not.toBeCloseTo(start.offset, 3);
    },
    cancelled(_start, at_cancel, after) {
      expect(after.offset).toBeCloseTo(at_cancel.offset, 9);
    },
    async fresh(page) {
      const g = await geometry(page);
      const from = { x: g.left + g.width * 0.5, y: g.top + g.pane_height * 0.3 };
      const before = await this.read(page);
      await drag(page, from, { x: from.x - 100, y: from.y });
      expect((await this.read(page)).offset).not.toBeCloseTo(before.offset, 3);
    },
  },
  {
    name: "price-axis scale",
    async prepare(page) {
      await goto_fixture(page);
      const g = await geometry(page);
      const from = { x: g.left + g.width + 10, y: g.top + g.pane_height * 0.4 };
      return {
        from,
        before: line(from, { x: from.x, y: from.y + 60 }, 6),
        after: line({ x: from.x, y: from.y + 60 }, { x: from.x, y: from.y + 140 }, 5),
      };
    },
    read: (page) => page.evaluate(() => {
      const scale = window.__chart.price_scale("right");
      return { range: scale.get_visible_range(), auto_scale: scale.options().auto_scale };
    }),
    started(start, mid) {
      expect(mid.auto_scale).toBe(false);
      expect(mid.range.to - mid.range.from).not.toBeCloseTo(start.range.to - start.range.from, 3);
    },
    cancelled(_start, at_cancel, after) {
      expect(after.range.from).toBeCloseTo(at_cancel.range.from, 9);
      expect(after.range.to).toBeCloseTo(at_cancel.range.to, 9);
    },
    async fresh(page) {
      // A scale session that was never ended would make the engine ignore this new start.
      const g = await geometry(page);
      const from = { x: g.left + g.width + 10, y: g.top + g.pane_height * 0.4 };
      const before = await this.read(page);
      await drag(page, from, { x: from.x, y: from.y - 80 });
      const after = await this.read(page);
      expect(after.range.to - after.range.from).not.toBeCloseTo(before.range.to - before.range.from, 3);
    },
  },
  {
    name: "drawing body drag",
    async prepare(page) {
      await goto_fixture(page);
      await page.evaluate(() => {
        const range = window.__chart.time_scale().get_visible_logical_range();
        const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
        const l1 = Math.floor(range.from + (range.to - range.from) * 0.6);
        const price = window.__chart.price_scale("right").get_visible_range();
        const level = price.from + (price.to - price.from) * 0.8;
        window.__drawing = window.__chart.add_drawing("trend_line", [
          { logical: l0, price: level },
          { logical: l1, price: level },
        ]);
      });
      await settle_frames(page);
      const from = await this.body_center(page);
      return {
        from,
        before: line(from, { x: from.x + 40, y: from.y + 30 }, 7),
        after: line({ x: from.x + 40, y: from.y + 30 }, { x: from.x + 90, y: from.y + 60 }, 5),
      };
    },
    async body_center(page) {
      const g = await geometry(page);
      const center = await page.evaluate(() => {
        const [a, b] = window.__drawing.points();
        const scale = window.__chart.time_scale();
        return {
          x: (scale.logical_to_coordinate(a.logical) + scale.logical_to_coordinate(b.logical)) / 2,
          y: window.__main.price_to_coordinate(a.price),
        };
      });
      return { x: g.left + center.x, y: g.top + center.y };
    },
    read: (page) => page.evaluate(() => ({
      points: window.__drawing.points(),
      dragging: window.__chart.wasm.drawing_drag_active(),
    })),
    started(start, mid) {
      expect(mid.dragging).toBe(true);
      expect(mid.points).not.toEqual(start.points);
    },
    cancelled(start, _at_cancel, after) {
      expect(after.dragging).toBe(false);
      expect(after.points).toEqual(start.points);
    },
    async fresh(page) {
      const before = await this.read(page);
      const from = await this.body_center(page);
      await drag(page, from, { x: from.x + 40, y: from.y + 30 });
      const after = await this.read(page);
      expect(after.dragging).toBe(false);
      expect(after.points).not.toEqual(before.points);
    },
  },
  {
    name: "trading line drag",
    async prepare(page) {
      await goto_fixture(page, "trading");
      await page.waitForFunction(() => window.__demo_catalogs?.lab.active_ids().includes("trading-bracket"));
      await settle_frames(page);
      await page.evaluate(() => {
        window.__intents = [];
        window.__chart.trading().subscribe_intents((intent) => window.__intents.push(intent));
      });
      const from = await this.line_point(page);
      return {
        from,
        before: line(from, { x: from.x, y: from.y - 40 }, 8),
        after: line({ x: from.x, y: from.y - 40 }, { x: from.x, y: from.y - 90 }, 5),
      };
    },
    async line_point(page) {
      const g = await geometry(page);
      const probe = await page.evaluate((width) => {
        const order = window.__chart.trading().state().orders.find((item) => item.id === "demo-target");
        const x = width - 200;
        const y = window.__main.price_to_coordinate(order.price);
        return { x, y, hit: window.__chart.trading().hit_at(x, y) };
      }, g.width);
      expect(probe.hit).toMatchObject({ id: "demo-target", kind: "order_line" });
      return { x: g.left + probe.x, y: g.top + probe.y };
    },
    read: (page) => page.evaluate(() => ({
      preview: window.__chart.trading().preview(),
      price: window.__chart.trading().state().orders.find((item) => item.id === "demo-target").price,
      intents: window.__intents.length,
    })),
    started(_start, mid) {
      expect(mid.preview).not.toBeNull();
    },
    cancelled(_start, _at_cancel, after) {
      expect(after.preview).toBeNull();
      expect(after.intents).toBe(0);
    },
    async fresh(page) {
      const from = await this.line_point(page);
      await drag(page, from, { x: from.x, y: from.y - 40 });
      const intents = await page.evaluate(() => window.__intents);
      expect(intents).toHaveLength(1);
      expect(intents[0]).toMatchObject({ action: "modify_order", order_id: "demo-target" });
    },
  },
  {
    name: "brush capture",
    async prepare(page) {
      await goto_fixture(page);
      await page.evaluate(() => window.__chart.set_drawing_tool("brush", { color: "#000000", width: 2 }));
      const g = await geometry(page);
      const from = { x: g.left + g.width * 0.3, y: g.top + g.pane_height * 0.3 };
      return {
        from,
        before: line(from, { x: from.x + 120, y: from.y + 60 }, 8),
        after: line({ x: from.x + 120, y: from.y + 60 }, { x: from.x + 220, y: from.y }, 6),
      };
    },
    read: (page) => page.evaluate(() => ({
      capturing: window.__chart.wasm.drawing_tool_capture_active(),
      drawings: window.__chart.drawings().length,
      tool: window.__chart.active_drawing_tool(),
    })),
    started(_start, mid) {
      expect(mid.capturing).toBe(true);
    },
    cancelled(_start, _at_cancel, after) {
      // The stroke is discarded; the armed tool stays armed for the next stroke.
      expect(after).toEqual({ capturing: false, drawings: 0, tool: "brush" });
    },
    async fresh(page) {
      const g = await geometry(page);
      const from = { x: g.left + g.width * 0.3, y: g.top + g.pane_height * 0.5 };
      await drag(page, from, { x: from.x + 160, y: from.y - 40 }, 10);
      const after = await this.read(page);
      expect(after.capturing).toBe(false);
      expect(after.drawings).toBe(1);
    },
  },
  {
    name: "pane separator resize",
    async prepare(page) {
      await goto_fixture(page);
      await page.evaluate(() => {
        const series = window.__chart.add_series("line", {});
        series.set_data(window.__data.map((bar) => ({ time: bar.time, value: bar.close })));
        series.move_to_pane(1);
      });
      await settle_frames(page);
      const from = await this.separator_point(page);
      return {
        from,
        before: line(from, { x: from.x, y: from.y + 40 }, 8),
        after: line({ x: from.x, y: from.y + 40 }, { x: from.x, y: from.y + 100 }, 5),
      };
    },
    async separator_point(page) {
      const g = await geometry(page);
      const y = await page.evaluate(() => Array.from(window.__chart.wasm.pane_separator_ys())[0]);
      return { x: g.left + g.width * 0.5, y: g.top + y };
    },
    read: (page) => page.evaluate(() => ({ separator: Array.from(window.__chart.wasm.pane_separator_ys())[0] })),
    started(start, mid) {
      expect(mid.separator).toBeGreaterThan(start.separator + 10);
    },
    cancelled(_start, at_cancel, after) {
      expect(after.separator).toBeCloseTo(at_cancel.separator, 9);
    },
    async fresh(page) {
      const before = await this.read(page);
      const from = await this.separator_point(page);
      await drag(page, from, { x: from.x, y: from.y - 60 });
      expect((await this.read(page)).separator).toBeLessThan(before.separator - 10);
    },
  },
];

const triggers = [
  {
    name: "window blur",
    fire: (page) => page.evaluate(() => window.dispatchEvent(new Event("blur"))),
  },
  {
    name: "the document turning hidden",
    fire: (page) => page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
      delete document.visibilityState;
    }),
  },
  {
    // A real capture release; the browser delivers lostpointercapture before the next pointer
    // event, so the continued drag below is what observes it.
    name: "lost pointer capture",
    fire: (page) => page.evaluate((selector) => {
      window.__lost_before = window.__lost_captures;
      document.querySelector(selector).releasePointerCapture(window.__pointer_id);
    }, OVERLAY),
    confirm: async (page) => {
      expect(await page.evaluate(() => window.__lost_captures - window.__lost_before)).toBe(1);
    },
  },
  {
    name: "pointercancel",
    fire: (page) => page.evaluate((selector) => {
      document.querySelector(selector).dispatchEvent(new PointerEvent("pointercancel", {
        pointerId: window.__pointer_id,
        pointerType: "mouse",
        bubbles: true,
      }));
    }, OVERLAY),
  },
  {
    name: "a host resize",
    fire: async (page) => {
      const viewport = page.viewportSize();
      await page.setViewportSize({ width: viewport.width - 100, height: viewport.height });
    },
    confirm: async (page, before) => {
      expect((await geometry(page)).overlay_width).toBe(before.overlay_width - 100);
    },
  },
  {
    name: "backend loss",
    fire: (page) => page.evaluate(() => window.dispatchEvent(new CustomEvent("aeris_charts-chart-backend-lost"))),
  },
];

for (const gesture of gestures) {
  for (const trigger of triggers) {
    test(`${trigger.name} cancels a ${gesture.name} mid-drag`, async ({ page }) => {
      const plan = await gesture.prepare(page);
      const start = await gesture.read(page);
      const before_geometry = await geometry(page);

      await page.mouse.move(plan.from.x, plan.from.y);
      await page.mouse.down();
      await move_through(page, plan.before);
      const mid = await gesture.read(page);
      gesture.started(start, mid);

      await trigger.fire(page);
      await settle_frames(page);
      const at_cancel = await gesture.read(page);
      // The button is still held: the cancelled gesture must not resume following it.
      await move_through(page, plan.after);
      await trigger.confirm?.(page, before_geometry);
      const after = await gesture.read(page);
      gesture.cancelled(start, at_cancel, after);

      await page.mouse.up();
      await settle_frames(page);
      gesture.cancelled(start, at_cancel, await gesture.read(page));
      expect(await page.evaluate(() => window.__clicks), "a past-slop drag release emits no click").toBe(0);

      await gesture.fresh(page);
    });
  }
}
