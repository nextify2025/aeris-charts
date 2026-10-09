import { createRequire } from "node:module";
import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";

const axe_path = createRequire(import.meta.url).resolve("axe-core/axe.min.js");
const wcag_tags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

async function assert_descriptions(page) {
  const descriptions = await page.evaluate(() =>
    [...document.querySelectorAll(".aeris_charts-a11y-layer[aria-describedby]")].map((layer) => {
      const id = layer.getAttribute("aria-describedby");
      const targets = document.querySelectorAll(`#${CSS.escape(id)}`);
      return { id, target_count: targets.length, owns_target: layer.contains(targets[0]) };
    }));
  expect(descriptions.length).toBeGreaterThanOrEqual(2);
  expect(new Set(descriptions.map(({ id }) => id)).size, JSON.stringify(descriptions)).toBe(descriptions.length);
  for (const description of descriptions) {
    expect(description.target_count, description.id).toBe(1);
    expect(description.owns_target, description.id).toBe(true);
  }
  return descriptions.map(({ id }) => id);
}

for (const theme of ["light", "dark"]) {
  for (const scenario of ["Financial", "General", "split grid", "chart-focused shortcuts panel", "390px mobile layout"]) {
    test(`${scenario} has no WCAG 2.1 A/AA violations in ${theme}`, async ({ page }) => {
      if (scenario === "390px mobile layout") await page.setViewportSize({ width: 390, height: 780 });
      const url = `/?theme=${theme}${scenario === "General" ? "&demo=general" : ""}`;
      await page.goto(url);
      await wait_for_chart(page, { grid: scenario === "split grid" });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator('meta[name="viewport"]')).toHaveAttribute(
        "content", "width=device-width, initial-scale=1, viewport-fit=cover",
      );
      await expect(page.locator("#chart_container")).toHaveCSS("touch-action", "none");
      for (const selector of [".market-summary", "#comparison_controls", "#series_grid", ".general-toolbar"]) {
        await expect(page.locator(selector)).toHaveAttribute("role", "group");
        await expect(page.locator(selector)).toHaveAttribute("aria-label", /.+/);
      }
      await expect(page.locator("#controls_strip")).toHaveAttribute("role", "region");
      await expect(page.locator("#controls_strip")).toHaveAttribute("aria-label", /.+/);

      if (scenario === "General") {
        await expect(page.locator(".general-chart-host .aeris_charts-a11y-layer").first()).toBeVisible();
        await page.waitForFunction(() => document.querySelectorAll(".general-chart-host .aeris_charts-a11y-layer").length >= 2);
        for (const host of await page.locator(".general-chart-host").all()) {
          await expect(host).toHaveAttribute("role", "group");
          await expect(host).toHaveAttribute("aria-label", /.+/);
        }
        console.log(`${scenario} ${theme} descriptions:`, await assert_descriptions(page));
      }
      if (scenario === "split grid") {
        await page.locator("#split_h").click();
        await page.waitForFunction(() => window.__grid.cells().length >= 2 &&
          window.__grid.cells().every((cell) => cell.chart?.backend?.() !== undefined));
        console.log(`${scenario} ${theme} descriptions:`, await assert_descriptions(page));
      }
      if (scenario === "chart-focused shortcuts panel") {
        await page.evaluate(() => window.__chart.accessibility().apply_options({ show_shortcuts: true }));
        await page.locator("#chart_container .aeris_charts-a11y-layer").first().focus();
        await page.keyboard.press("h");
        await expect(page.locator(".aeris_charts-a11y-shortcuts-panel").first()).toBeVisible();
      }
      if (scenario === "390px mobile layout") {
        for (const mode of ["financial", "general"]) {
          if (mode === "general") await page.locator("#demo_mode_general").click();
          const heading = page.locator(mode === "general" ? "#general_workspace h1" : ".mobile-heading");
          await expect(heading).toBeVisible();
          await expect(heading).toBeInViewport();
          const box = await heading.boundingBox();
          expect(box.width * box.height).toBeGreaterThan(0);
        }
        await page.locator("#demo_mode_financial").click();
      }

      await page.addScriptTag({ path: axe_path });
      const result = await page.evaluate(async (tags) => {
        const { violations, incomplete } = await window.axe.run(document, {
          runOnly: { type: "tag", values: tags },
        });
        return {
          violations: violations.map(({ id, nodes }) => ({
            id,
            nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
          })),
          aria_incomplete: incomplete.filter(({ id }) => id === "aria-prohibited-attr")
            .flatMap(({ nodes }) => nodes.map(({ target }) => target)),
        };
      }, wcag_tags);
      expect(result.violations, JSON.stringify(result, null, 2)).toEqual([]);
      expect(result.aria_incomplete).toEqual([]);
    });
  }
}
