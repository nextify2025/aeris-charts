import { execFileSync, spawn } from "node:child_process";
import { brotliCompressSync, gzipSync } from "node:zlib";
import { mkdtemp, readFile, readdir, rm, stat, writeFile, copyFile, mkdir } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { repository_root } from "./core.mjs";
import { dataset_metadata, metric } from "./shared.mjs";

const package_root = path.join(repository_root, "packages", "charts");
const executable = process.platform === "win32" ? "npm.cmd" : "npm";

function run(file, args, cwd) {
  const command = process.platform === "win32" && file.endsWith(".cmd") ? ["cmd.exe", ["/d", "/s", "/c", file, ...args]] : [file, args];
  return execFileSync(command[0], command[1], { cwd, encoding: "utf8", windowsHide: true, stdio: ["ignore", "pipe", "inherit"] });
}

// wasm-pack logs this line when it runs wasm-opt and only prints "Skipping wasm-opt" otherwise (for example on a
// platform it cannot fetch binaryen for), so its absence means the measured module is not the optimized one.
const wasm_opt_ran = /Optimizing wasm binaries with `wasm-opt`/;

export function assert_wasm_opt_ran(build_log) {
  if (!wasm_opt_ran.test(build_log)) throw new Error("the production build did not run wasm-opt, so the measured wasm is not the optimized module the package ships");
}

function run_logged(file, args, cwd) {
  const command = process.platform === "win32" && file.endsWith(".cmd") ? ["cmd.exe", ["/d", "/s", "/c", file, ...args]] : [file, args];
  return new Promise((resolve, reject) => {
    const child = spawn(command[0], command[1], { cwd, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    let log = "";
    for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk) => { log += chunk; process.stderr.write(chunk); });
    child.on("error", reject);
    child.on("close", (code) => code === 0 ? resolve(log) : reject(new Error(`${file} ${args.join(" ")} exited with status ${code}`)));
  });
}

async function build_package() {
  assert_wasm_opt_ran(await run_logged(executable, ["run", "build"], package_root));
}

async function compressed_metrics(prefix, filename, visibility = "public_candidate") {
  const bytes = await readFile(filename);
  return {
    [`${prefix}_raw_bytes`]: metric([bytes.length], "bytes", "lower_is_better", visibility, `Raw byte length of ${path.basename(filename)} from the production package build.`),
    [`${prefix}_gzip_bytes`]: metric([gzipSync(bytes, { level: 9 }).length], "bytes", "lower_is_better", visibility, `gzip level 9 byte length of ${path.basename(filename)} from the production package build.`),
    [`${prefix}_brotli_bytes`]: metric([brotliCompressSync(bytes).length], "bytes", "lower_is_better", visibility, `Node Brotli default-quality byte length of ${path.basename(filename)} from the production package build.`),
  };
}

async function declaration_bytes(directory) {
  let total = 0;
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) total += await declaration_bytes(filename);
    else if (entry.name.endsWith(".d.ts")) total += (await stat(filename)).size;
  }
  return total;
}

async function consumer_bundle(kind, source, temporary) {
  const entry = path.join(temporary, `${kind}.mjs`);
  const output = path.join(temporary, `${kind}.js`);
  await writeFile(entry, source);
  run(executable, ["exec", "--", "esbuild", entry, "--bundle", "--minify", "--format=esm", `--outfile=${output}`], package_root);
  return compressed_metrics(`consumer_${kind}_js_bundle`, output);
}

export async function prepare_browser_artifacts({ build = false } = {}) {
  if (build) await build_package();
  const demo_dist = path.join(repository_root, "examples", "web_demo", "dist");
  await mkdir(demo_dist, { recursive: true });
  await copyFile(path.join(package_root, "dist", "index.js"), path.join(demo_dist, "aeris_charts_financial.js"));
  await copyFile(path.join(package_root, "dist", "index.js.map"), path.join(demo_dist, "index.js.map"));
  await copyFile(path.join(package_root, "dist", "aeris_charts_wasm_bg.wasm"), path.join(demo_dist, "aeris_charts_wasm_bg.wasm"));
}

export function parse_pack_manifest(output) {
  const parsed = JSON.parse(output);
  if (!Array.isArray(parsed) || parsed.length !== 1 || !Number.isFinite(parsed[0].size) || !Number.isFinite(parsed[0].unpackedSize)) {
    throw new Error("npm pack did not return one package with finite size fields");
  }
  return parsed[0];
}

export async function measure_size({ build = true } = {}) {
  const started = performance.now();
  if (build) await build_package();
  const pack = parse_pack_manifest(run(executable, ["pack", "--json", "--dry-run"], package_root));
  const dist = path.join(package_root, "dist");
  const temporary = await mkdtemp(path.join(os.tmpdir(), "aeris_charts-bundle-"));
  try {
    await copyFile(path.join(dist, "index.js"), path.join(temporary, "package.js"));
    const package_url = "./package.js";
    const metrics = {
      npm_tarball_bytes: metric([pack.size], "bytes", "lower_is_better", "public_candidate", "npm pack --json --dry-run size for the publishable production tarball."),
      npm_unpacked_bytes: metric([pack.unpackedSize], "bytes", "lower_is_better", "public_candidate", "npm pack --json --dry-run unpackedSize for exactly the files npm would publish."),
      typescript_declarations_bytes: metric([await declaration_bytes(dist)], "bytes", "lower_is_better", "public_candidate", "Sum of .d.ts files in the production dist directory."),
      ...await compressed_metrics("javascript", path.join(dist, "index.js")),
      ...await compressed_metrics("wasm", path.join(dist, "aeris_charts_wasm_bg.wasm")),
      ...await consumer_bundle("minimal", `import { AerisChartsError } from ${JSON.stringify(package_url)}; console.log(AerisChartsError);`, temporary),
      ...await consumer_bundle("typical", `import { create_chart, init_wasm } from ${JSON.stringify(package_url)}; console.log(create_chart, init_wasm);`, temporary),
      ...await consumer_bundle("full", `import * as charts from ${JSON.stringify(package_url)}; console.log(charts);`, temporary),
    };
    return {
      id: "package-release-artifacts",
      version: 1,
      status: "passed",
      error: null,
      reproduction_command: "node benchmarks/benchmark.mjs size",
      dataset: dataset_metadata(0, 0, { series_count: 0, pane_count: 0 }),
      execution: { warmup_runs: 0, measured_runs: 1, benchmark_duration_ms: performance.now() - started, sampling_method: "filesystem bytes from the production npm build and npm pack manifest", forced_gc: false },
      capabilities: { npm_pack_manifest: true, brotli: true, consumer_bundle_excludes_external_wasm: true },
      metrics,
    };
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}
