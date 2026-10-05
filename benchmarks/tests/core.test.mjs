import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { base_environment, benchmark_root, build_provenance, compare_runs, evaluate_absolute_budgets, load_manifest, propose_size_budgets, public_summary, repository_root, validate_run, wasm_build_profile, wasm_opt_arguments, wasm_opt_version } from "../core.mjs";
import { assert_wasm_opt_ran, parse_pack_manifest } from "../size.mjs";
import { dataset_metadata, generate_ohlcv, metric, percentile, summarize } from "../shared.mjs";

function fixture() {
  return {
    schema_version: 1,
    product: { name: "aeris_charts-financial", version: "0.8.13" },
    source: { git_commit: "abc", git_branch: "main", git_tag: null, dirty_worktree: false },
    build: { profile: "release", logging: "default-no-verbose-debug", build_command: "bun run build", rustc_version: "rustc test", wasm_pack_version: "wasm-pack test", cargo_profile: "release", wasm_opt_version: "wasm-opt test", node_version: process.version, npm_version: "test", esbuild_version: "test", package_lock_sha256: "0".repeat(64), cargo_lock_sha256: "1".repeat(64), wasm_opt_args: ["-Oz"] },
    environment: { ...base_environment("official-benchmark-runner"), id: "official-test" },
    execution: { profile: "release", started_at: "2026-01-01T00:00:00.000Z", completed_at: "2026-01-01T00:01:00.000Z", clock: "performance.now monotonic in browser and Node; std::time::Instant monotonic in native Rust; wall time is metadata only", command: "benchmark release" },
    scenarios: [{
      id: "historical-candlestick-10k", version: 1, status: "passed", error: null,
      reproduction_command: "benchmark scenario historical-candlestick-10k",
      dataset: dataset_metadata(10_000, 7),
      execution: { warmup_runs: 1, measured_runs: 3, benchmark_duration_ms: 10, sampling_method: "test", forced_gc: false },
      capabilities: {},
      metrics: { set_data_api_ms: { availability: "measured", unit: "ms", direction: "lower_is_better", visibility: "public_candidate", methodology: "test", samples: [1, 2, 3], summary: summarize([1, 2, 3]) } },
    }],
  };
}

test("claims scan excludes split architecture evidence but still scans public documentation", async () => {
  const sandbox = await mkdtemp(path.join(os.tmpdir(), "aeris_charts-claims-"));
  try {
    const tools = path.join(sandbox, "benchmarks");
    await mkdir(tools);
    for (const file of ["benchmark.mjs", "core.mjs", "shared.mjs", "size.mjs"]) {
      await copyFile(path.join(benchmark_root, file), path.join(tools, file));
    }
    const public_docs = ["Readme.md", "docs/api/README.md", "docs/features/depth.md", "docs/development/contributing.md"];
    const internal_docs = [
      "docs/Architecture.md", "docs/architecture/data/storage.md", "docs/architecture/engine/input.md",
      "docs/architecture/rendering/frame.md", "docs/architecture/hosts/browser.md",
      "docs/development/performance.md", "docs/development/validation.md",
    ];
    for (const file of [...public_docs, ...internal_docs]) {
      await mkdir(path.dirname(path.join(sandbox, file)), { recursive: true });
      await writeFile(path.join(sandbox, file), "Measured frame cost 5 ms\n");
    }
    execFileSync("git", ["init", "--quiet"], { cwd: sandbox });
    execFileSync("git", ["add", "Readme.md", "docs"], { cwd: sandbox });
    execFileSync(process.execPath, [path.join(tools, "benchmark.mjs"), "claims"], { cwd: sandbox });
    const report = JSON.parse(await readFile(path.join(tools, "results", "unsupported-claims.json"), "utf8"));
    assert.deepEqual(report.findings.map(({ file }) => file).sort(), public_docs.sort());
  } finally {
    await rm(sandbox, { recursive: true, force: true });
  }
});

test("dataset generation is deterministic and preserves OHLC invariants", () => {
  const first = generate_ohlcv(100, 42);
  const second = generate_ohlcv(100, 42);
  assert.deepEqual([...first.close], [...second.close]);
  assert.notDeepEqual([...first.close], [...generate_ohlcv(100, 43).close]);
  for (let index = 0; index < first.times.length; index += 1) {
    assert.ok(first.high[index] >= first.open[index]);
    assert.ok(first.high[index] >= first.close[index]);
    assert.ok(first.low[index] <= first.open[index]);
    assert.ok(first.low[index] <= first.close[index]);
    assert.ok(first.high[index] >= first.low[index]);
  }
});

test("statistics use documented nearest-rank percentiles and retain outliers", () => {
  const sorted = [1, 2, 3, 4, 100];
  assert.equal(percentile(sorted, 0.50), 3);
  assert.equal(percentile(sorted, 0.95), 100);
  const summary = summarize(sorted);
  assert.equal(summary.max, 100);
  assert.equal(summary.count, 5);
  assert.ok(summary.standard_deviation > 0);
});

test("unsupported capabilities never acquire placeholder samples", () => {
  const unavailable = metric([], "ms", "lower_is_better", "public_candidate", "test capability", "unsupported");
  assert.deepEqual(unavailable.samples, []);
  assert.equal(unavailable.summary, null);
  assert.equal(unavailable.availability, "unsupported");
});

test("result validation rejects non-finite and negative duration samples", () => {
  assert.equal(validate_run(fixture()).schema_version, 1);
  const invalid = fixture();
  invalid.scenarios[0].metrics.set_data_api_ms.samples = [Number.NaN];
  assert.throws(() => validate_run(invalid), /non-finite/);
  const negative = fixture();
  negative.scenarios[0].metrics.set_data_api_ms.samples = [-1];
  assert.throws(() => validate_run(negative), /negative/);

  const unavailable_with_value = fixture();
  unavailable_with_value.scenarios[0].metrics.set_data_api_ms.availability = "unsupported";
  assert.throws(() => validate_run(unavailable_with_value), /unavailable metric carries a value/);

  const passed_without_metrics = fixture();
  passed_without_metrics.scenarios[0].metrics = {};
  assert.throws(() => validate_run(passed_without_metrics), /passed scenario has no metrics/);

  const inconsistent = fixture();
  inconsistent.scenarios[0].metrics.set_data_api_ms.summary.p50 = 999;
  assert.throws(() => validate_run(inconsistent), /inconsistent p50/);
});

test("comparison enforces scenario, dataset, and environment compatibility", () => {
  const baseline = fixture();
  const current = fixture();
  current.scenarios[0].metrics.set_data_api_ms = { ...current.scenarios[0].metrics.set_data_api_ms, samples: [2, 4, 6], summary: summarize([2, 4, 6]) };
  const compared = compare_runs(baseline, current, { thresholds: { "historical-candlestick-10k.set_data_api_ms.p50": { warning_percent: 20, fail_percent: 50 } } });
  assert.deepEqual(compared.budget_policy, { status: "ENFORCED", threshold_count: 1 });
  assert.equal(compared.comparisons[0].percentage_change, 100);
  assert.equal(compared.comparisons[0].status, "fail");
  const no_budget = compare_runs(baseline, baseline);
  assert.deepEqual(no_budget.budget_policy, { status: "NO ENFORCED BUDGET", threshold_count: 0 });
  current.scenarios[0].dataset.seed += 1;
  assert.equal(compare_runs(baseline, current).comparisons[0].status, "incompatible");
  current.scenarios[0].dataset.seed -= 1;
  current.environment.runtime_version = "different-browser-version";
  assert.equal(compare_runs(baseline, current).comparisons[0].environment_compatible, false);
  current.environment.runtime_version = baseline.environment.runtime_version;
  current.build.rustc_version = "different-toolchain";
  assert.equal(compare_runs(baseline, current).comparisons[0].build_compatible, false);
  assert.throws(() => compare_runs(baseline, baseline, { thresholds: { "historical-candlestick-10k.set_data_api_ms.p50": { warning_percent: 20, fail_percent: 10 } } }), /invalid warning\/failure budget/);
  assert.throws(() => compare_runs(baseline, baseline, { thresholds: { "misspelled.metric.p50": { warning_percent: 20, fail_percent: 50 } } }), /matched no comparable metric/);
});

test("absolute budgets enforce deterministic metrics without an environment baseline", () => {
  const run = fixture();
  const key = "historical-candlestick-10k.set_data_api_ms.p50";
  const passing = evaluate_absolute_budgets(run, { policy_version: 2, absolute_maximums: { [key]: 2 } });
  assert.deepEqual(passing.budget_policy, { status: "ENFORCED", threshold_count: 1 });
  assert.deepEqual(passing.evaluations[0], {
    key,
    scenario: "historical-candlestick-10k",
    metric: "set_data_api_ms",
    statistic: "p50",
    maximum: 2,
    current: 2,
    status: "pass",
    reason: null,
  });

  const failing = evaluate_absolute_budgets(run, { absolute_maximums: { [key]: 1 } });
  assert.equal(failing.evaluations[0].status, "fail");
  assert.deepEqual(
    evaluate_absolute_budgets(run, { absolute_maximums: { "package-release-artifacts.wasm_raw_bytes.p50": 1 } }).budget_policy,
    { status: "NOT APPLICABLE", threshold_count: 0 },
  );
  assert.throws(
    () => evaluate_absolute_budgets(run, { absolute_maximums: { "historical-candlestick-10k.missing.p50": 1 } }),
    /no metric/,
  );
  assert.throws(
    () => evaluate_absolute_budgets(run, { absolute_maximums: { "malformed": 1 } }),
    /<scenario>\.<metric>\.p50/,
  );
  assert.throws(
    () => evaluate_absolute_budgets(run, { absolute_maximums: { [key]: -1 } }),
    /finite non-negative/,
  );
});

test("public summary includes only measured public candidates from official clean releases", () => {
  const run = fixture();
  run.scenarios[0].metrics.internal_only = metric([7], "count", "informational", "internal", "test");
  run.scenarios[0].metrics.unsupported_public = metric([], "ms", "lower_is_better", "public_candidate", "test", "unsupported");
  const summary = public_summary(run);
  assert.equal(summary.release, "0.8.13");
  assert.ok(summary.scenarios["historical-candlestick-10k@1"].metrics.set_data_api_ms);
  assert.equal(summary.scenarios["historical-candlestick-10k@1"].metrics.internal_only, undefined);
  assert.equal(summary.scenarios["historical-candlestick-10k@1"].metrics.unsupported_public, undefined);
  const dirty = fixture();
  dirty.source.dirty_worktree = true;
  assert.throws(() => public_summary(dirty), /clean worktree/);
});

test("environment capture is privacy-safe and pack output is validated", () => {
  const environment = base_environment("local");
  assert.equal("hostname" in environment, false);
  assert.equal("username" in environment, false);
  assert.equal(parse_pack_manifest('[{"size":123,"unpackedSize":456}]').size, 123);
  assert.throws(() => parse_pack_manifest("[]"), /one package/);
});

test("scenario registry and JSON schema remain versioned and complete", async () => {
  const manifest = await load_manifest();
  assert.equal(manifest.manifest_version, 1);
  assert.equal(new Set(manifest.scenarios.map(({ id }) => id)).size, manifest.scenarios.length);
  for (const scenario of manifest.scenarios) {
    assert.ok(scenario.version >= 1);
    assert.ok(scenario.profiles.length > 0);
  }
  const budgets = JSON.parse(await readFile(path.join(benchmark_root, "budgets.json"), "utf8"));
  const by_id = new Map(manifest.scenarios.map((scenario) => [scenario.id, scenario]));
  for (const key of Object.keys(budgets.absolute_maximums ?? {})) {
    const scenario_id = key.slice(0, key.indexOf("."));
    const scenario = by_id.get(scenario_id);
    assert.ok(scenario, "absolute budget references registered scenario " + scenario_id);
    assert.ok(
      scenario.profiles.includes("release"),
      "absolute-budget scenario " + scenario_id + " must remain in the release profile",
    );
  }
  const schema = JSON.parse(await readFile(path.join(benchmark_root, "schema", "result-v1.schema.json"), "utf8"));
  assert.equal(schema.properties.schema_version.const, 1);
  assert.ok(schema.required.includes("environment"));
  assert.ok(schema.$defs.summary.required.includes("p95"));
  assert.equal(schema.$defs.dataset.properties.seed.maximum, 0xffff_ffff);
});

test("wasm build provenance reads the wasm-opt flags wasm-pack actually uses from the crate metadata", async () => {
  const cargo_toml = await readFile(path.join(repository_root, "crates", "aeris_charts_wasm", "Cargo.toml"), "utf8");
  const declared = (section) => JSON.parse(new RegExp(`\\[package\\.metadata\\.wasm-pack\\.profile\\.${section}\\]\\s*wasm-opt = (\\[.*\\])`).exec(cargo_toml)?.[1] ?? "null");
  const build_script = JSON.parse(await readFile(path.join(repository_root, "packages", "charts", "package.json"), "utf8")).scripts["build:wasm"];
  const profile = wasm_build_profile(build_script);
  const expected = declared(profile === "release" ? "release" : "custom");
  assert.ok(expected.length > 0 && expected.includes("--enable-simd"));
  assert.deepEqual(declared("custom") ?? expected, declared("release"), "the custom-profile wasm-opt flags cannot drift from the release list");
  const provenance = await build_provenance("package");
  assert.deepEqual(provenance.wasm_opt_args, expected);
  assert.equal(provenance.cargo_profile, profile);
  assert.ok(provenance.wasm_opt_version === null || /^wasm-opt version \d+/.test(provenance.wasm_opt_version));
  const native = await build_provenance("native");
  assert.deepEqual([native.wasm_opt_args, native.wasm_opt_version, native.cargo_profile], [[], null, "release"]);

  const metadata = { packages: [{ name: "aeris_charts_wasm", metadata: { "wasm-pack": { profile: { release: { "wasm-opt": ["-Oz"] }, custom: { "wasm-opt": ["-Os"] } } } } }] };
  assert.deepEqual(wasm_opt_arguments(metadata, "release"), ["-Oz"]);
  assert.deepEqual(wasm_opt_arguments(metadata, "wasm-release"), ["-Os"]);
  assert.throws(() => wasm_opt_arguments({ packages: [] }, "release"), /no wasm-opt arguments/);
  assert.throws(() => wasm_opt_arguments({ packages: [{ name: "aeris_charts_wasm", metadata: { "wasm-pack": { profile: { release: { "wasm-opt": [] } } } } }] }, "release"), /no wasm-opt arguments/);
  assert.equal(wasm_build_profile("wasm-pack build ../../crates/aeris_charts_wasm --target web"), "release");
  assert.equal(wasm_build_profile("wasm-pack build ../../crates/aeris_charts_wasm --target web --profile wasm-release --out-dir pkg"), "wasm-release");
});

test("wasm-opt version falls back to the newest wasm-pack cache binary only when none is on PATH", { skip: process.platform === "win32" }, async () => {
  const cache = await mkdtemp(path.join(os.tmpdir(), "aeris_charts-wasm-pack-cache-"));
  const original_path = process.env.PATH;
  try {
    const bin = path.join(cache, "wasm-opt-abc", "bin");
    await mkdir(bin, { recursive: true });
    await writeFile(path.join(bin, "wasm-opt"), "#!/bin/sh\necho 'wasm-opt version 117 (version_117)'\n");
    await chmod(path.join(bin, "wasm-opt"), 0o755);
    const empty = path.join(cache, "empty");
    await mkdir(empty);
    process.env.PATH = empty;
    assert.equal(await wasm_opt_version(cache), "wasm-opt version 117 (version_117)");
    assert.equal(await wasm_opt_version(path.join(cache, "missing")), null);
  } finally {
    process.env.PATH = original_path;
    await rm(cache, { recursive: true, force: true });
  }
});

test("release build metadata stays on the release channel and the size step requires a wasm-opt run", () => {
  assert.equal(validate_run(fixture()).build.cargo_profile, "release");
  const debug = fixture();
  debug.build.profile = "wasm-release";
  assert.throws(() => validate_run(debug), /invalid release build metadata/);
  const unnamed = fixture();
  unnamed.build.cargo_profile = "";
  assert.throws(() => validate_run(unnamed), /invalid wasm build provenance/);
  const legacy = fixture();
  delete legacy.build.cargo_profile;
  delete legacy.build.wasm_opt_version;
  assert.equal(validate_run(legacy).schema_version, 1);
  assert.doesNotThrow(() => assert_wasm_opt_ran("[INFO]: Optimizing wasm binaries with `wasm-opt`...\n[INFO]: :-) Done in 1s"));
  assert.throws(() => assert_wasm_opt_ran("[INFO]: Skipping wasm-opt because it is not supported on this platform"), /did not run wasm-opt/);
});

test("only exceeded size ceilings are raised, to observed plus headroom rounded up, with the evidence recorded beside them", () => {
  const run = fixture();
  const bytes = (value) => ({ availability: "measured", unit: "bytes", direction: "lower_is_better", visibility: "public_candidate", methodology: "test", samples: [value], summary: summarize([value]) });
  run.scenarios[0] = { ...run.scenarios[0], id: "package-release-artifacts", metrics: { wasm_raw_bytes: bytes(4_739_804), wasm_brotli_bytes: bytes(1_175_083), npm_tarball_bytes: bytes(1_000_000), javascript_raw_bytes: bytes(412_622) } };
  const budgets = { policy_version: 3, thresholds: {}, absolute_maximums: {
    "package-release-artifacts.wasm_raw_bytes.p50": 3_000_000,
    "package-release-artifacts.wasm_brotli_bytes.p50": 810_000,
    "package-release-artifacts.npm_tarball_bytes.p50": 1_000_000,
    "package-release-artifacts.javascript_raw_bytes.p50": 620_000,
    "general-dashboard-100k.startup_ms.p50": 2000,
  } };
  const proposal = propose_size_budgets(run, budgets, { levers: ["example lever"], product_tradeoff: "example tradeoff" });
  assert.equal(proposal.absolute_maximums["package-release-artifacts.wasm_raw_bytes.p50"], 5_080_000);
  assert.equal(proposal.absolute_maximums["package-release-artifacts.wasm_brotli_bytes.p50"], 1_260_000);
  assert.equal(proposal.absolute_maximums["package-release-artifacts.npm_tarball_bytes.p50"], 1_000_000, "a ceiling that holds (observed equals it) is not moved");
  assert.equal(proposal.absolute_maximums["package-release-artifacts.javascript_raw_bytes.p50"], 620_000, "a ceiling with headroom is not tightened");
  assert.equal(proposal.absolute_maximums["general-dashboard-100k.startup_ms.p50"], 2000);
  assert.equal(proposal.policy_version, 4);
  assert.equal(proposal.rationale.length, 1);
  assert.equal(proposal.rationale[0].headroom_percent, 7);
  assert.deepEqual(Object.keys(proposal.rationale[0].raised), ["package-release-artifacts.wasm_raw_bytes.p50", "package-release-artifacts.wasm_brotli_bytes.p50"]);
  assert.deepEqual(proposal.rationale[0].raised["package-release-artifacts.wasm_raw_bytes.p50"], { from: 3_000_000, to: 5_080_000 });
  assert.equal(proposal.rationale[0].observed["package-release-artifacts.wasm_raw_bytes.p50"], 4_739_804);
  assert.equal(proposal.rationale[0].toolchain.wasm_opt, "wasm-opt test");
  assert.equal(proposal.rationale[0].product_tradeoff, "example tradeoff");
  assert.equal(budgets.policy_version, 3, "the input budgets are not mutated");
  assert.equal(evaluate_absolute_budgets(run, proposal).evaluations.filter(({ status }) => status === "fail").length, 0, "the proposal passes the run it was derived from");
  assert.throws(() => propose_size_budgets(fixture(), budgets), /no passed package-release-artifacts scenario/);
});
