#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import {
  base_environment,
  benchmark_root,
  build_provenance,
  compare_runs,
  evaluate_absolute_budgets,
  load_manifest,
  markdown_report,
  product_version,
  promote_baseline,
  propose_size_budgets,
  public_summary,
  read_json,
  repository_root,
  source_provenance,
  validate_run,
  write_run,
} from "./core.mjs";
import { measure_size, prepare_browser_artifacts } from "./size.mjs";

const command = process.argv[2] ?? "help";
const args = process.argv.slice(3);

function run(file, file_args, cwd) {
  if (process.platform === "win32" && file.endsWith(".cmd")) execFileSync("cmd.exe", ["/d", "/s", "/c", file, ...file_args], { cwd, stdio: "inherit", windowsHide: true });
  else execFileSync(file, file_args, { cwd, stdio: "inherit", windowsHide: true });
}

async function exists(filename) {
  try { await stat(filename); return true; } catch { return false; }
}

async function ensure_dependencies(browser) {
  const package_modules = path.join(repository_root, "packages", "charts", "node_modules");
  if (!(await exists(path.join(package_modules, ".bin")))) run("bun", ["install", "--frozen-lockfile"], path.join(repository_root, "packages", "charts"));
  if (browser) {
    const demo_modules = path.join(repository_root, "examples", "web_demo", "node_modules");
    if (!(await exists(path.join(demo_modules, ".bin")))) run("bun", ["install", "--frozen-lockfile"], path.join(repository_root, "examples", "web_demo"));
  }
}

function option(name, fallback = null) {
  const index = args.indexOf(name);
  return index < 0 ? fallback : args[index + 1];
}

async function build_run(profile, scenarios, execution_command) {
  const started_at = new Date().toISOString();
  const source = source_provenance();
  if (profile === "release" && source.dirty_worktree) throw new Error("official release benchmarks refuse a dirty worktree");
  const size_scenarios = scenarios.filter(({ kind }) => kind === "size");
  const native_scenarios = scenarios.filter(({ kind }) => kind === "native");
  const browser_scenarios = scenarios.filter(({ kind }) => !["size", "native"].includes(kind));
  if (size_scenarios.length > 0 || browser_scenarios.length > 0) await ensure_dependencies(browser_scenarios.length > 0);
  const measured = [];
  if (native_scenarios.length > 0) {
    const { measure_native } = await import("./native.mjs");
    measured.push(...native_scenarios.map(measure_native));
  }
  if (size_scenarios.length > 0) {
    measured.push(await measure_size({ build: true }));
    if (browser_scenarios.length > 0) await prepare_browser_artifacts();
  } else if (browser_scenarios.length > 0) {
    await prepare_browser_artifacts({ build: true });
  }
  let browser = null;
  if (browser_scenarios.length > 0) {
    const { run_browser_scenarios } = await import("./browser.mjs");
    const result = await run_browser_scenarios(browser_scenarios);
    measured.push(...result.scenarios);
    browser = result.browser;
  }
  const environment = base_environment();
  if (native_scenarios.length > 0 && browser_scenarios.length === 0) {
    environment.runtime = "native-rust";
    environment.runtime_version = execFileSync("rustc", ["--version"], { cwd: repository_root, encoding: "utf8", windowsHide: true }).trim();
  }
  if (browser) Object.assign(environment, {
    gpu: browser.gpu ?? null,
    gpu_vendor: browser.gpu_vendor ?? null,
    gpu_driver: null,
    runtime: browser.runtime,
    runtime_version: browser.runtime_version,
    viewport: browser.viewport,
    device_pixel_ratio: browser.device_pixel_ratio,
    refresh_rate_hz: null,
  });
  const run_result = validate_run({
    schema_version: 1,
    product: { name: "aeris_charts-financial", version: await product_version() },
    source,
    build: await build_provenance(native_scenarios.length > 0 && size_scenarios.length === 0 && browser_scenarios.length === 0 ? "native" : "package"),
    environment,
    execution: {
      profile,
      started_at,
      completed_at: new Date().toISOString(),
      clock: "performance.now monotonic in browser and Node; std::time::Instant monotonic in native Rust; wall time is metadata only",
      command: execution_command,
    },
    scenarios: measured,
  });
  const filename = await write_run(run_result);
  console.log(filename);
  const absolute_budgets = evaluate_absolute_budgets(
    run_result,
    await read_json(path.join(benchmark_root, "budgets.json")),
  );
  if (absolute_budgets.budget_policy.status === "ENFORCED") {
    console.log(JSON.stringify(absolute_budgets, null, 2));
  }
  if (absolute_budgets.evaluations.some(({ status }) => status === "fail")) process.exitCode = 1;
  if (measured.some(({ status }) => status === "failed")) process.exitCode = 1;
  return filename;
}

async function run_profile(profile) {
  const manifest = await load_manifest();
  let scenarios = manifest.scenarios.filter((scenario) => scenario.profiles.includes(profile));
  const duration = Number(option("--duration-ms", "0"));
  if (duration > 0) scenarios = scenarios.map((scenario) => scenario.kind === "soak" ? { ...scenario, duration_ms: duration } : scenario);
  await build_run(profile, scenarios, `node benchmarks/benchmark.mjs ${profile}${duration > 0 ? ` --duration-ms ${duration}` : ""}`);
}

async function run_scenario(id) {
  const manifest = await load_manifest();
  let scenario = manifest.scenarios.find((candidate) => candidate.id === id);
  if (!scenario) throw new Error(`unknown scenario: ${id}`);
  const duration = Number(option("--duration-ms", "0"));
  if (duration > 0 && scenario.duration_ms !== undefined) scenario = { ...scenario, duration_ms: duration };
  await build_run("scenario", [scenario], `node benchmarks/benchmark.mjs scenario ${id}${duration > 0 ? ` --duration-ms ${duration}` : ""}`);
}

async function compare(baseline_file, current_file) {
  const budgets = await read_json(path.join(benchmark_root, "budgets.json"));
  const result = compare_runs(await read_json(baseline_file), await read_json(current_file), budgets);
  console.log(JSON.stringify(result, null, 2));
  if (result.comparisons.some(({ status }) => status === "fail")) process.exitCode = 1;
}

async function report(filename) {
  const run_result = await read_json(filename);
  const output = path.join(path.dirname(filename), `${path.basename(filename, ".json")}.md`);
  await writeFile(output, markdown_report(run_result));
  console.log(output);
}

async function publish_summary(filename) {
  const run_result = await read_json(filename);
  Object.defineProperty(run_result, "__filename", { value: filename, enumerable: false });
  const output = path.join(path.dirname(filename), "benchmark-public.json");
  await writeFile(output, `${JSON.stringify(public_summary(run_result), null, 2)}\n`, { flag: "wx" });
  console.log(output);
}

async function claims() {
  const files = execFileSync("git", ["ls-files"], { cwd: repository_root, encoding: "utf8", windowsHide: true }).split(/\r?\n/).filter(Boolean);
  const numeric = /(?:~|\b)\d+(?:\.\d+)?(?:\s*-\s*\d+(?:\.\d+)?)?\s*(?:x|fps|kb|mb|gb|ms|µs|us|points?\/sec|updates?\/sec)\b/i;
  const performance = /\b(?:measured|fastest|faster|slower|throughput|charges?|costs?|memory|heap|overhead|budget|frame|load|update)\b/i;
  const vague_claim = /\b(?:high[ -]?performance|lightweight|fastest|low[ -]?latency|low[ -]?memory|small bundle|millions? of)\b/i;
  const findings = [];
  for (const relative of files) {
    if (/\.(?:png|ttf|wasm|lock)$/i.test(relative)
      || relative.startsWith("benchmarks/")
      || relative === "AGENTS.md"
      || relative === "docs/Architecture.md"
      // 架构拆分后的内部契约与证据页延续原总览的排除范围；公共 API 文档仍参与扫描。
      || relative.startsWith("docs/architecture/")
      || relative === "docs/development/performance.md"
      || relative === "docs/development/validation.md"
      || relative.includes("/tests/")
      || relative.endsWith("/examples/perf_gate.rs")) continue;
    const text = await readFile(path.join(repository_root, relative), "utf8").catch(() => "");
    for (const [index, line] of text.split(/\r?\n/).entries()) {
      const trimmed = line.trim();
      const is_documentation = /^(?:\/\/|\/\*|\*|#|<!--|[A-Za-z]|"description")/.test(trimmed);
      const match = trimmed.match(numeric)?.[0] ?? trimmed.match(vague_claim)?.[0];
      if (is_documentation && match && (vague_claim.test(trimmed) || performance.test(trimmed)) && !/\b(?:MSAA|default budget)\b|PowerPreference::|lightweight-charts/i.test(trimmed)) {
        findings.push({ file: relative.replaceAll("\\", "/"), line: index + 1, match, status: "unsupported_without_linked_benchmark_result", text: trimmed });
      }
    }
  }
  const output = path.join(benchmark_root, "results", "unsupported-claims.json");
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify({ generated_at: new Date().toISOString(), findings }, null, 2)}\n`);
  console.log(output);
}

// Prints the proposed budgets.json (new ceilings plus an appended rationale entry); never writes the file, so the
// diff is reviewed and committed deliberately. The product tradeoff is mandatory: a size increase must explain it.
async function rebudget(size_result) {
  const product_tradeoff = option("--tradeoff");
  if (!size_result || !product_tradeoff) throw new Error("usage: rebudget <size-result.json> --tradeoff <text> [--lever <text>]... [--headroom-percent 7]");
  const levers = args.flatMap((argument, index) => argument === "--lever" ? [args[index + 1]] : []);
  const budgets = await read_json(path.join(benchmark_root, "budgets.json"));
  console.log(JSON.stringify(propose_size_budgets(await read_json(size_result), budgets, { headroom_percent: Number(option("--headroom-percent", "7")), levers, product_tradeoff }), null, 2));
}

function help() {
  console.log(`Aeris Charts evidence benchmark CLI

  node benchmarks/benchmark.mjs test
  node benchmarks/benchmark.mjs smoke
  node benchmarks/benchmark.mjs nightly
  node benchmarks/benchmark.mjs release [--duration-ms N]
  node benchmarks/benchmark.mjs soak [--duration-ms N]
  node benchmarks/benchmark.mjs scenario <id> [--duration-ms N]
  node benchmarks/benchmark.mjs native
  node benchmarks/benchmark.mjs size
  node benchmarks/benchmark.mjs rebudget <size-result.json> --tradeoff <text> [--lever <text>]... [--headroom-percent 7]
  node benchmarks/benchmark.mjs compare <baseline.json> <current.json>
  node benchmarks/benchmark.mjs report <result.json>
  node benchmarks/benchmark.mjs public <official-release-result.json>
  node benchmarks/benchmark.mjs baseline <official-release-result.json>
  node benchmarks/benchmark.mjs claims`);
}

try {
  if (["smoke", "nightly", "release", "soak"].includes(command)) await run_profile(command);
  else if (command === "scenario") await run_scenario(args[0]);
  else if (command === "size") await build_run("scenario", [(await load_manifest()).scenarios.find(({ kind }) => kind === "size")], "node benchmarks/benchmark.mjs size");
  else if (command === "native") await build_run("scenario", [(await load_manifest()).scenarios.find(({ kind }) => kind === "native")], "node benchmarks/benchmark.mjs native");
  else if (command === "rebudget") await rebudget(args[0]);
  else if (command === "compare") await compare(args[0], args[1]);
  else if (command === "report") await report(args[0]);
  else if (command === "public") await publish_summary(args[0]);
  else if (command === "baseline") console.log(await promote_baseline(args[0]));
  else if (command === "claims") await claims();
  else if (command === "test") run(process.execPath, ["--test", "benchmarks/tests/core.test.mjs"], repository_root);
  else help();
} catch (error) {
  console.error(error.stack ?? String(error));
  process.exitCode = 1;
}
