import { test, expect } from "@playwright/test";

test("study calendar rejects invalid spans atomically and schemas use revision two", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line");
    source.set_data([{ time: 1_700_000_000, value: 10 }, { time: 1_700_000_060, value: 11 }]);
    const study = chart.add_sma(source, 2);
    chart.set_study_calendar([{ startTime: 1_699_999_999, endTime: 1_700_000_120, sessionId: 7 }]);
    const before = study.data().at(-1)?.value;
    let invalidCalendar;
    try {
      chart.set_study_calendar([
        { startTime: 1_699_999_999, endTime: 1_700_000_120, sessionId: 7 },
        { startTime: 1_700_000_100, endTime: 1_700_000_180, sessionId: 8 },
      ]);
    } catch (error) {
      invalidCalendar = error.code;
    }
    let invalidBinding;
    try {
      chart.study_annotations(study);
    } catch (error) {
      invalidBinding = error.code;
    }
    chart.clear_study_calendar();
    return {
      before,
      after: study.data().at(-1)?.value,
      invalidCalendar,
      invalidBinding,
      schema: chart.indicator_schema("sma", 2),
    };
  });
  expect(result).toMatchObject({
    before: 10.5,
    after: 10.5,
    invalidCalendar: "invalid_options",
    invalidBinding: "unsupported_operation",
    // The fork's schema revision 4 (upstream 2) renamed the choice list to `options`.
    schema: { revision: 4, parameters: [
      { name: "source", parameter_type: "source" },
      { name: "period", parameter_type: "integer", default: 2 },
    ] },
  });
  expect(result.schema.parameters.every(({ options }) => options === undefined)).toBe(true);
});
