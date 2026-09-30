import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../..", import.meta.url));
const ci = readFileSync(`${root}/.github/workflows/ci.yml`, "utf8");
const publish = readFileSync(`${root}/.github/workflows/publish.yml`, "utf8");

// Every release-relevant build installs the same wasm-pack: its bundled wasm-opt decides the
// shipped WASM bytes, so an unpinned or differently pinned installer moves the size budgets and
// the published artifact without any source change. The benchmark workflows use this exact pin.
const WASM_PACK_INSTALL = "cargo install wasm-pack --locked --version 0.15.0";

function assertWasmPackPinned(source, workflow) {
  const installs = [...source.matchAll(/- name: Install wasm-pack\r?\n\s+run: ([^\r\n]+)/g)];
  assert.ok(installs.length > 0, `${workflow} must install wasm-pack through a named step`);
  for (const [, command] of installs) {
    assert.equal(command.trim(), WASM_PACK_INSTALL, `${workflow} must pin wasm-pack to the audited version`);
  }
}

function verify(ciSource, publishSource) {
  assert.match(ciSource, /Run required portable browser suite[\s\S]*npx playwright test/,
    "portable Playwright must remain a required browser step");
  assert.doesNotMatch(ciSource, /Run required portable browser suite[\s\S]{0,180}continue-on-error: true/,
    "portable Playwright cannot continue on error");
  assert.match(ciSource, /AERIS_CHARTS_PERF_STRICT: "1"/,
    "the configured release perf budget must block CI");
  assert.match(ciSource, /Enforce production artifact size budgets[\s\S]{0,180}node benchmarks\/benchmark\.mjs size/,
    "deterministic package and WASM size budgets must block CI");
  assert.doesNotMatch(ciSource, /Enforce production artifact size budgets[\s\S]{0,180}continue-on-error: true/,
    "artifact size budgets cannot continue on error");
  assert.match(ciSource, /machine-sensitive[\s\S]{0,220}continue-on-error: true/,
    "machine-calibrated evidence must remain non-authoritative");
  assertWasmPackPinned(ciSource, "ci.yml");
  assertWasmPackPinned(publishSource, "publish.yml");
  assert.match(publishSource, /tags: \["v\*"\]/,
    "version tags must trigger publication");
  assert.match(publishSource, /actions: read[\s\S]*contents: read[\s\S]*packages: write/,
    "publication must be able to verify CI, read the release source, and publish packages");
  assert.match(publishSource, /registry-url: https:\/\/npm\.pkg\.github\.com/,
    "the scoped browser package must publish through GitHub Packages");
  assert.match(publishSource, /scope: "@aeristerminal"/,
    "GitHub Packages publication must use the Aeris Terminal scope");
  assert.match(publishSource, /NODE_AUTH_TOKEN: \$\{\{ secrets\.GITHUB_TOKEN \}\}/,
    "GitHub Packages publication must use the workflow token");
  assert.match(publishSource, /Verify source passed CI[\s\S]*actions\/workflows\/ci\.yml\/runs/,
    "publication must require successful CI for the release source");
  assert.match(publishSource, /npm publish --tag latest/,
    "publication must update the latest package tag");
}

verify(ci, publish);
for (const [brokenCi, brokenPublish] of [
  [ci.replace("AERIS_CHARTS_PERF_STRICT: \"1\"", "AERIS_CHARTS_PERF_STRICT: \"0\""), publish],
  [ci.replace("node benchmarks/benchmark.mjs size", "node benchmarks/benchmark.mjs test"), publish],
  [ci.replace("id: portable-browser-suite", "id: portable-browser-suite\n        continue-on-error: true"), publish],
  [ci, publish.replace("actions/workflows/ci.yml/runs", "actions/workflows/missing.yml/runs")],
  [ci, publish.replace("https://npm.pkg.github.com", "https://registry.npmjs.org")],
  [ci, publish.replace("npm publish --tag latest", "npm publish")],
  [ci.replace(WASM_PACK_INSTALL, "cargo install wasm-pack --locked"), publish],
  [ci.replace(WASM_PACK_INSTALL, "curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh"), publish],
  [ci.replace(WASM_PACK_INSTALL, WASM_PACK_INSTALL.replace("0.15.0", "0.14.0")), publish],
  [ci.replaceAll("Install wasm-pack", "Install wasm toolchain"), publish],
  [ci, publish.replace(WASM_PACK_INSTALL, "cargo install wasm-pack --locked --version 0.14.0")],
  [ci, publish.replace(WASM_PACK_INSTALL, "curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh")],
]) {
  assert.throws(() => verify(brokenCi, brokenPublish), "a simulated release-gate regression was not detected");
}
console.log("release gate policy and failure simulations OK");
