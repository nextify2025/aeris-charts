import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { access, copyFile, mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { summarize } from "./shared.mjs";

export const benchmark_root = path.dirname(fileURLToPath(import.meta.url));
export const repository_root = path.dirname(benchmark_root);

function command(file, args, fallback = null) {
  try {
    const executable = process.platform === "win32" && file.endsWith(".cmd") ? process.env.ComSpec ?? "cmd.exe" : file;
    const executable_args = executable === file ? args : ["/d", "/s", "/c", file, ...args];
    return execFileSync(executable, executable_args, { cwd: repository_root, encoding: "utf8", windowsHide: true, stdio: ["ignore", "pipe", "ignore"] }).trim() || fallback;
  } catch {
    return fallback;
  }
}

export async function read_json(filename) {
  return JSON.parse(await readFile(filename, "utf8"));
}

export async function load_manifest() {
  return read_json(path.join(benchmark_root, "scenarios.json"));
}

export async function product_version() {
  return (await read_json(path.join(repository_root, "packages", "charts", "package.json"))).version;
}

// wasm-pack reads its wasm-opt flags from the crate metadata section named after the build mode:
// `release` for a plain `wasm-pack build`, `custom` for `--profile <name>`.
export function wasm_opt_arguments(cargo_metadata, cargo_profile) {
  const crate = cargo_metadata?.packages?.find(({ name }) => name === "aeris_charts_wasm");
  const section = crate?.metadata?.["wasm-pack"]?.profile?.[cargo_profile === "release" ? "release" : "custom"];
  const args = section?.["wasm-opt"];
  if (!Array.isArray(args) || args.length === 0 || !args.every((argument) => typeof argument === "string")) {
    throw new Error(`aeris_charts_wasm has no wasm-opt arguments for Cargo profile ${cargo_profile} (or \`cargo metadata\` could not run); wasm-pack would ship an unoptimized or default-optimized module`);
  }
  return args;
}

// Cargo profile wasm-pack builds with: the `--profile <name>` in the package's build:wasm script, else `release`.
export function wasm_build_profile(build_wasm_script) {
  return /--profile[ =]([A-Za-z0-9_-]+)/.exec(build_wasm_script)?.[1] ?? "release";
}

// wasm-pack runs a `wasm-opt` found on PATH and only otherwise downloads its own binaryen into
// `<cache>/.wasm-pack`, so the version that optimized the module is the PATH one if present,
// else the newest wasm-opt found under that cache. Null when neither can be found or run.
export async function wasm_opt_version(cache_root = wasm_pack_cache_root()) {
  const on_path = command("wasm-opt", ["--version"]);
  if (on_path) return on_path;
  const executable = process.platform === "win32" ? "wasm-opt.exe" : "wasm-opt";
  const found = [];
  async function scan(directory, depth) {
    for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory() && depth < 4) await scan(filename, depth + 1);
      else if (entry.name === executable) found.push({ filename, modified: (await stat(filename)).mtimeMs });
    }
  }
  await scan(cache_root, 0);
  found.sort((left, right) => right.modified - left.modified);
  return found.length > 0 ? command(found[0].filename, ["--version"]) : null;
}

function wasm_pack_cache_root() {
  if (process.platform === "win32") return path.join(process.env.LOCALAPPDATA ?? os.homedir(), ".wasm-pack");
  if (process.platform === "darwin") return path.join(os.homedir(), "Library", "Caches", ".wasm-pack");
  return path.join(process.env.XDG_CACHE_HOME ?? path.join(os.homedir(), ".cache"), ".wasm-pack");
}

export async function build_provenance(kind = "package") {
  const lock = await readFile(path.join(repository_root, "packages", "charts", "package-lock.json"));
  const cargo_lock = await readFile(path.join(repository_root, "Cargo.lock"));
  const esbuild = await read_json(path.join(repository_root, "packages", "charts", "node_modules", "esbuild", "package.json")).catch(() => null);
  const native = kind === "native";
  const cargo_profile = native ? "release" : wasm_build_profile((await read_json(path.join(repository_root, "packages", "charts", "package.json"))).scripts?.["build:wasm"] ?? "");
  const wasm_opt_args = native ? [] : wasm_opt_arguments(JSON.parse(command("cargo", ["metadata", "--no-deps", "--format-version", "1"]) ?? "null"), cargo_profile);
  return {
    profile: "release",
    logging: "default-no-verbose-debug",
    build_command: native ? "cargo run --quiet --release -p aeris_charts_native --example evidence_bench" : "npm run build",
    rustc_version: command("rustc", ["--version"]),
    wasm_pack_version: command("wasm-pack", ["--version"]),
    node_version: process.version,
    npm_version: command(process.platform === "win32" ? "npm.cmd" : "npm", ["--version"]),
    esbuild_version: esbuild?.version ?? null,
    package_lock_sha256: createHash("sha256").update(lock).digest("hex"),
    cargo_lock_sha256: createHash("sha256").update(cargo_lock).digest("hex"),
    cargo_profile,
    wasm_opt_version: native ? null : await wasm_opt_version(),
    wasm_opt_args,
  };
}

export function source_provenance() {
  return {
    git_commit: command("git", ["rev-parse", "HEAD"]),
    git_branch: command("git", ["branch", "--show-current"]),
    git_tag: command("git", ["describe", "--tags", "--exact-match"]),
    dirty_worktree: command("git", ["status", "--porcelain"], "") !== "",
  };
}

export function base_environment(classification = process.env.AERIS_CHARTS_BENCH_ENV_CLASS ?? "local") {
  if (!["local", "shared-ci", "official-benchmark-runner"].includes(classification)) {
    throw new Error(`invalid environment classification: ${classification}`);
  }
  const cpus = os.cpus();
  return {
    classification,
    id: process.env.AERIS_CHARTS_BENCH_ENV_ID ?? classification,
    os: os.platform(),
    os_version: os.release(),
    architecture: os.arch(),
    cpu: cpus[0]?.model ?? null,
    logical_cpu_count: Math.max(1, cpus.length),
    physical_cpu_count: null,
    total_ram_bytes: os.totalmem(),
    gpu: null,
    gpu_vendor: null,
    gpu_driver: null,
    runtime: "node",
    runtime_version: process.version,
    viewport: null,
    device_pixel_ratio: null,
    refresh_rate_hz: null,
  };
}

function assert_record(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`);
}

function safe_segment(value, label) {
  const sanitized = String(value).replace(/[^a-zA-Z0-9_.-]/g, "-");
  if (!sanitized || sanitized === "." || sanitized === "..") throw new Error(`invalid ${label}`);
  return sanitized;
}

export function validate_run(run) {
  assert_record(run, "result");
  if (run.schema_version !== 1) throw new Error(`unsupported schema_version ${run.schema_version}`);
  if (run.product?.name !== "aeris_charts-financial" || !run.product.version) throw new Error("invalid product metadata");
  if (run.build?.profile !== "release" || run.build?.logging !== "default-no-verbose-debug" || !run.build?.build_command || !/^[a-f0-9]{64}$/.test(run.build?.package_lock_sha256 ?? "") || !/^[a-f0-9]{64}$/.test(run.build?.cargo_lock_sha256 ?? "") || !Array.isArray(run.build?.wasm_opt_args)) throw new Error("invalid release build metadata");
  // Optional so older results stay readable; `profile` above is the evidence channel, `cargo_profile` names the Cargo profile that built the wasm.
  if ((run.build.cargo_profile !== undefined && (typeof run.build.cargo_profile !== "string" || run.build.cargo_profile === "")) || (run.build.wasm_opt_version !== undefined && run.build.wasm_opt_version !== null && typeof run.build.wasm_opt_version !== "string")) throw new Error("invalid wasm build provenance");
  assert_record(run.source, "source");
  if (typeof run.source.dirty_worktree !== "boolean") throw new Error("invalid source metadata");
  assert_record(run.environment, "environment");
  if (!["local", "shared-ci", "official-benchmark-runner"].includes(run.environment.classification) || !run.environment.id) throw new Error("invalid environment metadata");
  if (!Number.isInteger(run.environment.logical_cpu_count) || run.environment.logical_cpu_count < 1 || !Number.isFinite(run.environment.total_ram_bytes) || run.environment.total_ram_bytes < 1) throw new Error("invalid environment capacity");
  if (run.environment.viewport !== null && (!Number.isInteger(run.environment.viewport?.width) || run.environment.viewport.width < 1 || !Number.isInteger(run.environment.viewport?.height) || run.environment.viewport.height < 1)) throw new Error("invalid viewport metadata");
  if (run.environment.device_pixel_ratio !== null && (!Number.isFinite(run.environment.device_pixel_ratio) || run.environment.device_pixel_ratio <= 0)) throw new Error("invalid device pixel ratio");
  assert_record(run.execution, "execution");
  if (!["smoke", "nightly", "release", "soak", "scenario"].includes(run.execution.profile) || !run.execution.command) throw new Error("invalid execution metadata");
  if (run.execution.clock !== "performance.now monotonic in browser and Node; std::time::Instant monotonic in native Rust; wall time is metadata only") throw new Error("invalid clock metadata");
  if (!Number.isFinite(Date.parse(run.execution.started_at)) || !Number.isFinite(Date.parse(run.execution.completed_at))) throw new Error("invalid execution timestamps");
  if (!Array.isArray(run.scenarios) || run.scenarios.length === 0) throw new Error("result has no scenarios");
  const identities = new Set();
  for (const scenario of run.scenarios) {
    const identity = `${scenario.id}@${scenario.version}`;
    if (!/^[a-z0-9-]+$/.test(scenario.id ?? "") || !Number.isInteger(scenario.version) || scenario.version < 1) throw new Error(`${identity}: invalid identity`);
    if (identities.has(identity)) throw new Error(`duplicate scenario ${identity}`);
    identities.add(identity);
    if (!["passed", "failed", "unsupported"].includes(scenario.status)) throw new Error(`${identity}: invalid status`);
    if (scenario.status === "failed" && !scenario.error) throw new Error(`${identity}: failed result has no error`);
    if (!scenario.reproduction_command) throw new Error(`${identity}: missing reproduction command`);
    assert_record(scenario.dataset, `${identity}.dataset`);
    if (!Number.isInteger(scenario.dataset.generator_version) || !Number.isInteger(scenario.dataset.seed) || scenario.dataset.seed < 0 || scenario.dataset.seed > 0xffff_ffff || !Number.isInteger(scenario.dataset.points) || scenario.dataset.points < 0 || !Number.isInteger(scenario.dataset.series_count) || scenario.dataset.series_count < 0 || !Number.isInteger(scenario.dataset.pane_count) || scenario.dataset.pane_count < 0) throw new Error(`${identity}: invalid dataset metadata`);
    assert_record(scenario.dataset.configuration, `${identity}.dataset.configuration`);
    assert_record(scenario.execution, `${identity}.execution`);
    if (!Number.isInteger(scenario.execution.warmup_runs) || scenario.execution.warmup_runs < 0 || !Number.isInteger(scenario.execution.measured_runs) || scenario.execution.measured_runs < 0 || !Number.isFinite(scenario.execution.benchmark_duration_ms) || scenario.execution.benchmark_duration_ms < 0 || !scenario.execution.sampling_method) throw new Error(`${identity}: invalid scenario execution metadata`);
    assert_record(scenario.metrics, `${identity}.metrics`);
    assert_record(scenario.capabilities, `${identity}.capabilities`);
    if (scenario.status === "passed" && Object.keys(scenario.metrics).length === 0) throw new Error(`${identity}: passed scenario has no metrics`);
    for (const [name, value] of Object.entries(scenario.metrics)) {
      assert_record(value, `${identity}.${name}`);
      if (!["measured", "unsupported", "not_applicable"].includes(value.availability)) throw new Error(`${identity}.${name}: invalid availability`);
      if (!value.unit || !["lower_is_better", "higher_is_better", "informational"].includes(value.direction) || !["internal", "public_candidate"].includes(value.visibility) || !value.methodology) throw new Error(`${identity}.${name}: invalid metric metadata`);
      if (!Array.isArray(value.samples)) throw new Error(`${identity}.${name}: samples must be an array`);
      if (value.availability === "measured" && value.samples.length === 0) throw new Error(`${identity}.${name}: measured metric has no samples`);
      if (value.availability !== "measured" && ((value.samples?.length ?? 0) !== 0 || value.summary !== null)) throw new Error(`${identity}.${name}: unavailable metric carries a value`);
      for (const sample of value.samples ?? []) {
        if (!Number.isFinite(sample)) throw new Error(`${identity}.${name}: non-finite sample`);
        if ((value.unit === "ms" || value.unit === "us" || value.unit === "bytes") && sample < 0 && !/(delta|growth|change)/.test(name)) {
          throw new Error(`${identity}.${name}: impossible negative sample`);
        }
      }
      if (value.availability === "measured" && (value.summary === null || value.summary?.count !== value.samples.length)) throw new Error(`${identity}.${name}: invalid summary`);
      if (value.availability === "measured") {
        const expected = summarize(value.samples);
        for (const [statistic, expected_value] of Object.entries(expected)) {
          const actual = value.summary[statistic];
          const tolerance = Number.EPSILON * Math.max(1, Math.abs(expected_value)) * 4;
          if (!Number.isFinite(actual) || Math.abs(actual - expected_value) > tolerance) throw new Error(`${identity}.${name}: inconsistent ${statistic}`);
        }
      }
    }
  }
  return run;
}

export async function write_run(run) {
  validate_run(run);
  const version = safe_segment(run.product.version, "product version");
  const environment = safe_segment(run.environment.id, "environment id");
  const directory = path.join(benchmark_root, "results", `v${version}`, environment);
  await mkdir(directory, { recursive: true });
  const stamp = run.execution.started_at.replace(/[:.]/g, "-");
  const commit = run.source.git_commit?.slice(0, 12) ?? "unknown";
  const filename = path.join(directory, `${stamp}-${commit}-${run.execution.profile}.json`);
  await writeFile(filename, `${JSON.stringify(run, null, 2)}\n`, { flag: "wx" });
  return filename;
}

function comparable(left, right) {
  return left.version === right.version
    && left.dataset?.generator_version === right.dataset?.generator_version
    && left.dataset?.seed === right.dataset?.seed
    && JSON.stringify(left.dataset?.configuration) === JSON.stringify(right.dataset?.configuration)
    && left.dataset?.points === right.dataset?.points
    && left.dataset?.series_count === right.dataset?.series_count
    && left.dataset?.pane_count === right.dataset?.pane_count;
}

function comparable_environment(left, right) {
  const fields = ["classification", "id", "os", "os_version", "architecture", "cpu", "logical_cpu_count", "physical_cpu_count", "total_ram_bytes", "gpu", "gpu_vendor", "gpu_driver", "runtime", "runtime_version", "device_pixel_ratio", "refresh_rate_hz"];
  return fields.every((field) => left[field] === right[field])
    && JSON.stringify(left.viewport) === JSON.stringify(right.viewport);
}

export function compare_runs(baseline, current, budgets = { thresholds: {} }) {
  validate_run(baseline);
  validate_run(current);
  assert_record(budgets, "budgets");
  assert_record(budgets.thresholds, "budgets.thresholds");
  const configured_thresholds = new Set(Object.keys(budgets.thresholds));
  const matched_thresholds = new Set();
  const baseline_by_id = new Map(baseline.scenarios.map((scenario) => [scenario.id, scenario]));
  const comparisons = [];
  for (const scenario of current.scenarios) {
    const before = baseline_by_id.get(scenario.id);
    if (!before) continue;
    const scenario_compatible = comparable(before, scenario);
    const environment_compatible = comparable_environment(baseline.environment, current.environment);
    const build_compatible = JSON.stringify(baseline.build) === JSON.stringify(current.build);
    const compatible = scenario_compatible && environment_compatible && build_compatible;
    for (const [metric_name, metric] of Object.entries(scenario.metrics)) {
      const old_metric = before.metrics[metric_name];
      if (metric.availability !== "measured" || old_metric?.availability !== "measured") continue;
      const baseline_value = old_metric.summary.p50;
      const current_value = metric.summary.p50;
      const absolute_change = current_value - baseline_value;
      const percentage_change = baseline_value === 0 ? null : absolute_change / Math.abs(baseline_value) * 100;
      const key = `${scenario.id}.${metric_name}.p50`;
      const threshold = budgets.thresholds?.[key] ?? null;
      if (threshold !== null) matched_thresholds.add(key);
      if (threshold !== null && (!Number.isFinite(threshold.warning_percent) || !Number.isFinite(threshold.fail_percent) || threshold.warning_percent < 0 || threshold.fail_percent < threshold.warning_percent)) {
        throw new Error(`${key}: invalid warning/failure budget`);
      }
      let status = compatible ? "pass" : "incompatible";
      if (compatible && threshold !== null && percentage_change !== null) {
        const regression = metric.direction === "higher_is_better" ? -percentage_change : percentage_change;
        status = regression > threshold.fail_percent ? "fail" : regression > threshold.warning_percent ? "warning" : "pass";
      }
      comparisons.push({ scenario: scenario.id, scenario_version: scenario.version, metric: metric_name, statistic: "p50", direction: metric.direction, baseline: baseline_value, current: current_value, absolute_change, percentage_change, scenario_compatible, environment_compatible, build_compatible, compatible, status });
    }
  }
  const unmatched = [...configured_thresholds].filter((key) => !matched_thresholds.has(key));
  if (unmatched.length > 0) throw new Error(`budget thresholds matched no comparable metric: ${unmatched.join(", ")}`);
  return {
    schema_version: 1,
    baseline_commit: baseline.source.git_commit,
    current_commit: current.source.git_commit,
    budget_policy: {
      status: configured_thresholds.size === 0 ? "NO ENFORCED BUDGET" : "ENFORCED",
      threshold_count: configured_thresholds.size,
    },
    comparisons,
  };
}

export function evaluate_absolute_budgets(run, budgets = { absolute_maximums: {} }) {
  validate_run(run);
  assert_record(budgets, "budgets");
  assert_record(budgets.absolute_maximums, "budgets.absolute_maximums");
  const scenarios = new Map(run.scenarios.map((scenario) => [scenario.id, scenario]));
  const evaluations = [];
  for (const [key, maximum] of Object.entries(budgets.absolute_maximums)) {
    if (!Number.isFinite(maximum) || maximum < 0) {
      throw new Error(`${key}: absolute maximum must be a finite non-negative number`);
    }
    const first_separator = key.indexOf(".");
    const last_separator = key.lastIndexOf(".");
    if (first_separator <= 0 || last_separator <= first_separator || key.slice(last_separator + 1) !== "p50") {
      throw new Error(`${key}: absolute budget key must be <scenario>.<metric>.p50`);
    }
    const scenario_id = key.slice(0, first_separator);
    const metric_name = key.slice(first_separator + 1, last_separator);
    const scenario = scenarios.get(scenario_id);
    if (!scenario) continue;
    if (scenario.status !== "passed") {
      evaluations.push({ key, scenario: scenario_id, metric: metric_name, statistic: "p50", maximum, current: null, status: "fail", reason: `scenario_${scenario.status}` });
      continue;
    }
    const metric = scenario.metrics[metric_name];
    if (!metric) throw new Error(`${key}: absolute budget matched a scenario but no metric`);
    if (metric.availability !== "measured" || !Number.isFinite(metric.summary?.p50)) {
      evaluations.push({ key, scenario: scenario_id, metric: metric_name, statistic: "p50", maximum, current: null, status: "fail", reason: `metric_${metric.availability}` });
      continue;
    }
    const current = metric.summary.p50;
    evaluations.push({ key, scenario: scenario_id, metric: metric_name, statistic: "p50", maximum, current, status: current <= maximum ? "pass" : "fail", reason: null });
  }
  return {
    policy_version: budgets.policy_version ?? null,
    budget_policy: {
      status: evaluations.length === 0 ? "NOT APPLICABLE" : "ENFORCED",
      threshold_count: evaluations.length,
    },
    evaluations,
  };
}

// Replacement package-size ceilings from one measured `package-release-artifacts` run. Only a ceiling the run exceeds
// moves: to the observed p50 plus `headroom_percent`, rounded up to the next 10,000 bytes (the Phase 2 reset carried
// 6.4-8.5% headroom). Passing ceilings and every non-package maximum are copied unchanged, so a re-baseline never
// silently tightens or loosens what still holds. The appended `rationale` entry records the observed values, the
// toolchain and the stated product tradeoff; `budgets.json` ignores keys it does not evaluate, so the evidence stays
// beside the numbers it justifies.
export function propose_size_budgets(run, budgets, { headroom_percent = 7, levers = [], product_tradeoff = "" } = {}) {
  validate_run(run);
  const size = run.scenarios.find(({ id, status }) => id === "package-release-artifacts" && status === "passed");
  if (!size) throw new Error("the run has no passed package-release-artifacts scenario");
  const observed = {};
  const raised = {};
  const absolute_maximums = { ...budgets.absolute_maximums };
  for (const [key, maximum] of Object.entries(absolute_maximums)) {
    if (!key.startsWith("package-release-artifacts.")) continue;
    const value = size.metrics[key.split(".")[1]]?.summary?.p50;
    if (!Number.isFinite(value)) throw new Error(`${key}: the run has no measured value`);
    observed[key] = value;
    if (value <= maximum) continue;
    absolute_maximums[key] = Math.ceil(Math.ceil(value * (100 + headroom_percent) / 100) / 10_000) * 10_000;
    raised[key] = { from: maximum, to: absolute_maximums[key] };
  }
  const policy_version = (budgets.policy_version ?? 0) + 1;
  const rationale = [...(budgets.rationale ?? []), {
    policy_version,
    commit: run.source.git_commit,
    dirty_worktree: run.source.dirty_worktree,
    headroom_percent,
    observed,
    raised,
    toolchain: { rustc: run.build.rustc_version, wasm_pack: run.build.wasm_pack_version, wasm_opt: run.build.wasm_opt_version ?? null, cargo_profile: run.build.cargo_profile ?? null, wasm_opt_args: run.build.wasm_opt_args },
    levers,
    product_tradeoff,
  }];
  return { ...budgets, policy_version, absolute_maximums, rationale };
}

function render_value(value) {
  return value === null ? "n/a" : Number.isInteger(value) ? String(value) : value.toFixed(3);
}

export function markdown_report(run, comparison = null) {
  validate_run(run);
  const lines = [
    "# Aeris Charts benchmark report",
    "",
    `Version: ${run.product.version}`,
    `Commit: ${run.source.git_commit ?? "unknown"}${run.source.dirty_worktree ? " (dirty)" : ""}`,
    `Environment: ${run.environment.id} (${run.environment.classification})`,
    `Runtime: ${run.environment.runtime} ${run.environment.runtime_version}`,
    `Build: ${run.build.build_command}; ${run.build.rustc_version ?? "unknown rustc"}; wasm-pack ${run.build.wasm_pack_version ?? "unknown"}`,
    `CPU: ${run.environment.cpu ?? "unknown"}`,
    `GPU: ${run.environment.gpu ?? "unknown"}`,
    "",
  ];
  for (const scenario of run.scenarios) {
    lines.push(`## ${scenario.id}@${scenario.version}`, "", `Status: ${scenario.status}`, "", "| Metric | Unit | p50 | p95 | p99 | Method |", "|---|---:|---:|---:|---:|---|");
    for (const [name, value] of Object.entries(scenario.metrics)) {
      lines.push(`| ${name} | ${value.unit} | ${render_value(value.summary?.p50 ?? null)} | ${render_value(value.summary?.p95 ?? null)} | ${render_value(value.summary?.p99 ?? null)} | ${value.methodology.replaceAll("|", "\\|")} |`);
    }
    lines.push("", `Reproduce: \`${scenario.reproduction_command}\``, "");
  }
  if (comparison) {
    lines.push("## Regression comparison", "", "| Scenario | Metric | Baseline | Current | Change | Status |", "|---|---|---:|---:|---:|---|");
    for (const row of comparison.comparisons) {
      lines.push(`| ${row.scenario} | ${row.metric} | ${render_value(row.baseline)} | ${render_value(row.current)} | ${row.percentage_change === null ? "n/a" : `${row.percentage_change.toFixed(2)}%`} | ${row.status} |`);
    }
  }
  return `${lines.join("\n")}\n`;
}

export function public_summary(run) {
  validate_run(run);
  if (run.execution.profile !== "release") throw new Error("public summaries require the release profile");
  if (run.environment.classification !== "official-benchmark-runner") throw new Error("public summaries require an official-benchmark-runner environment");
  if (run.source.dirty_worktree) throw new Error("public summaries require a clean worktree");
  const scenarios = {};
  for (const scenario of run.scenarios) {
    if (scenario.status !== "passed") continue;
    const metrics = {};
    for (const [name, value] of Object.entries(scenario.metrics)) {
      if (value.visibility === "public_candidate" && value.availability === "measured") metrics[name] = { unit: value.unit, summary: value.summary, methodology: value.methodology };
    }
    if (Object.keys(metrics).length > 0) scenarios[`${scenario.id}@${scenario.version}`] = { dataset: scenario.dataset, reproduction_command: scenario.reproduction_command, metrics };
  }
  return {
    schema_version: 1,
    release: run.product.version,
    commit: run.source.git_commit,
    measured_at: run.execution.completed_at,
    environment_id: run.environment.id,
    environment: run.environment,
    build: run.build,
    source_result: path.basename(run.__filename ?? "benchmark-result.json"),
    scenarios,
  };
}

export async function promote_baseline(result_filename) {
  const run = validate_run(await read_json(result_filename));
  if (run.execution.profile !== "release" || run.environment.classification !== "official-benchmark-runner" || run.source.dirty_worktree) {
    throw new Error("only clean official release results may become baselines");
  }
  const directory = path.join(benchmark_root, "baselines", `v${safe_segment(run.product.version, "product version")}`);
  await mkdir(directory, { recursive: true });
  const destination = path.join(directory, `${safe_segment(run.environment.id, "environment id")}.json`);
  try {
    await access(destination, constants.F_OK);
    throw new Error(`baseline already exists: ${destination}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await copyFile(result_filename, destination);
  return destination;
}
