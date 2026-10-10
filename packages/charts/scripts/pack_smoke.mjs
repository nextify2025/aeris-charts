/**
 * `bun run test:pack` — publish-readiness smoke test.
 *
 * Packs the package exactly as npm would for publish, installs the tarball into a scratch dir,
 * and asserts the installed artifact is complete and importable:
 *   1. `npm pack` produces a tarball containing JavaScript, types, WebAssembly, the portable
 *      design system, LICENSE, and NOTICE (third-party attributions).
 *   2. `npm install <tarball>` into an empty consumer dir.
 *   3. The installed core module imports in Node (side-effect-free) and exposes both naming styles.
 *   4. `dist/aeris_charts_wasm_bg.wasm` is present inside the installed package (non-trivial size).
 *
 * Node cannot *run* create_chart (browser-only wasm fetch + DOM) — this test deliberately checks
 * only that importing does not throw and the artifact set is complete.
 */
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const pkg_dir = fileURLToPath(new URL("..", import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), "aeris_charts-pack-smoke-"));

// When invoked from `npm publish --dry-run` (prepublishOnly), the parent leaks its dry-run
// config into our nested npm calls — strip it so the inner `npm pack` really writes a tarball.
const env = { ...process.env };
delete env.npm_config_dry_run;

// `npm_execpath` names whichever package manager started this script; only npm's own CLI can be
// re-run through node (under `bun run` it is the bun binary, so `npm` is called by name instead).
const npm_cli = /npm-cli\.[cm]?js$/.test(process.env.npm_execpath ?? "") ? process.env.npm_execpath : null;

const run = (cmd, args, cwd) => {
  if (cmd === "npm" && npm_cli) {
    return execFileSync(process.execPath, [npm_cli, ...args], { cwd, env, encoding: "utf8" }).trim();
  }
  return execFileSync(cmd, args, { cwd, env, encoding: "utf8" }).trim();
};

try {
  // 1. Pack and inspect the tarball file list.
  const tgz = run("npm", ["pack", "--silent"], pkg_dir);
  const files = run("tar", ["-tf", join(pkg_dir, tgz)], pkg_dir).split(/\r?\n/);
  for (const required of [
    "package/dist/index.js",
    "package/dist/index.d.ts",
    "package/dist/react.js",
    "package/dist/react.d.ts",
    "package/dist/aeris_charts_wasm_bg.wasm",
    "package/dist/aeris_charts.css",
    "package/LICENSE",
    "package/NOTICE",
  ]) {
    assert.ok(files.includes(required), `tarball is missing ${required}`);
  }
  assert.ok(
    !files.some((f) => f.startsWith("package/dist/src/")),
    "stale dist/src/*.d.ts duplicates leaked into the tarball (run bun run clean first)",
  );

  // 2. Install the tarball into a scratch consumer.
  writeFileSync(join(scratch, "package.json"), JSON.stringify({ name: "pack-smoke", type: "module" }));
  run("npm", ["install", "--silent", "--no-audit", "--no-fund", join(pkg_dir, tgz)], scratch);

  // 3. Import the installed module (must be side-effect-free) and check the API surface.
  const installed_package = join(scratch, "node_modules", "@aeristerminal", "aeris-charts");
  const entry = join(installed_package, "dist", "index.js");
  const mod = await import(pathToFileURL(entry).href);
  assert.equal(typeof mod.create_chart, "function", "create_chart not exported");
  assert.equal(typeof mod.createChart, "function", "createChart not exported");
  assert.equal(typeof mod.init_wasm, "function", "init_wasm not exported");
  assert.equal(typeof mod.initWasm, "function", "initWasm not exported");

  const installed_js = readFileSync(entry, "utf8");
  for (const forbidden of ["../pkg/", "../../crates/", "examples/web_demo", "benchmarks/"]) {
    assert.ok(!installed_js.includes(forbidden), `published runtime leaked repository-only path: ${forbidden}`);
  }

  // 4. The wasm binary shipped with real content.
  const wasm = statSync(
    join(installed_package, "dist", "aeris_charts_wasm_bg.wasm"),
  );
  assert.ok(wasm.size > 100_000, `wasm binary suspiciously small (${wasm.size} bytes)`);

  const pkg = JSON.parse(
    readFileSync(join(installed_package, "package.json"), "utf8"),
  );
  assert.equal(pkg.license, "AGPL-3.0-only");
  assert.equal(pkg.exports["./react"].import, "./dist/react.js");
  assert.equal(pkg.exports["./wasm"], "./dist/aeris_charts_wasm_bg.wasm");

  console.log(`pack smoke OK: ${tgz} (${files.length} files, wasm ${(wasm.size / 1024).toFixed(0)} kB)`);
  rmSync(join(pkg_dir, tgz), { force: true });
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
