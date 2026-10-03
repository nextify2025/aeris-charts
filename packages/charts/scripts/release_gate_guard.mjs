import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../..", import.meta.url));
const ci = readFileSync(`${root}/.github/workflows/ci.yml`, "utf8");
const publish = readFileSync(`${root}/.github/workflows/publish.yml`, "utf8");
// The benchmark workflows build the same package, so their wasm-pack is held to the same pin.
const benchmarkWorkflows = Object.fromEntries(
  ["benchmark-nightly.yml", "benchmark-release.yml", "metrics-smoke.yml"].map((name) => [
    name,
    readFileSync(`${root}/.github/workflows/${name}`, "utf8"),
  ]),
);

// The Rust toolchain is an exact release in rust-toolchain.toml, and every workflow installs that
// release: a floating `stable` let a new Rust release add a Clippy lint to an unchanged tree.
const RUST_RELEASE = readFileSync(`${root}/rust-toolchain.toml`, "utf8").match(/^channel\s*=\s*"([^"]+)"/m)?.[1];
assert.match(RUST_RELEASE ?? "", /^\d+\.\d+\.\d+$/, "rust-toolchain.toml must pin an exact Rust release, not a channel");

function assertRustToolchainPinned(source, workflow) {
  const steps = source.split(/\r?\n(?=\s*- )/).filter((step) => step.includes("dtolnay/rust-toolchain@"));
  assert.ok(steps.length > 0, `${workflow} must install Rust through dtolnay/rust-toolchain`);
  for (const step of steps) {
    const toolchain = step.match(/^\s+toolchain:\s*(\S+)/m)?.[1];
    assert.ok(
      toolchain === RUST_RELEASE || toolchain?.startsWith(`${RUST_RELEASE}-`),
      `${workflow} must install Rust ${RUST_RELEASE} like rust-toolchain.toml, not ${toolchain ?? "the floating default"}`,
    );
  }
}

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

function verify(ciSource, publishSource, benchmarkSources = benchmarkWorkflows) {
  assert.match(ciSource, /Run required portable browser suite[\s\S]*bunx playwright test/,
    "portable Playwright must remain a required browser step");
  assert.doesNotMatch(ciSource, /Run required portable browser suite[\s\S]{0,180}continue-on-error: true/,
    "portable Playwright cannot continue on error");
  assert.match(ciSource, /AERIS_CHARTS_PERF_STRICT: "1"/,
    "the configured release perf budget must block CI");
  assert.match(ciSource, /Enforce production artifact size budgets[\s\S]{0,180}node benchmarks\/benchmark\.mjs size/,
    "deterministic package and WASM size budgets must block CI");
  assert.doesNotMatch(ciSource, /Enforce production artifact size budgets[\s\S]{0,180}continue-on-error: true/,
    "artifact size budgets cannot continue on error");
  // Beyond the named steps below, no step of either workflow may install an unpinned wasm-pack.
  for (const [name, source] of [["ci.yml", ciSource], ["publish.yml", publishSource]]) {
    assert.doesNotMatch(source, /wasm-pack\/installer\/init\.sh|cargo install wasm-pack --locked(?! --version 0\.15\.0)/,
      `${name} cannot install an unpinned wasm-pack (it also selects the binaryen that optimizes the shipped module)`);
  }
  assert.match(ciSource, /machine-sensitive[\s\S]{0,220}continue-on-error: true/,
    "machine-calibrated evidence must remain non-authoritative");
  assertWasmPackPinned(ciSource, "ci.yml");
  assertWasmPackPinned(publishSource, "publish.yml");
  assertRustToolchainPinned(ciSource, "ci.yml");
  assertRustToolchainPinned(publishSource, "publish.yml");
  for (const [name, source] of Object.entries(benchmarkSources)) {
    assertWasmPackPinned(source, name);
    assertRustToolchainPinned(source, name);
  }
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
  [ci.replaceAll(`toolchain: ${RUST_RELEASE}`, "toolchain: stable"), publish],
  [ci.replace(`          toolchain: ${RUST_RELEASE}\n`, ""), publish],
  [ci, publish.replace(`toolchain: ${RUST_RELEASE}`, "toolchain: 1.98.1")],
  [ci, publish.replace(WASM_PACK_INSTALL, "cargo install wasm-pack --locked --version 0.14.0")],
  [ci, publish.replace(WASM_PACK_INSTALL, "curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh")],
]) {
  assert.throws(() => verify(brokenCi, brokenPublish), "a simulated release-gate regression was not detected");
}
for (const name of Object.keys(benchmarkWorkflows)) {
  const broken = {
    ...benchmarkWorkflows,
    [name]: benchmarkWorkflows[name].replace(WASM_PACK_INSTALL, "cargo install wasm-pack --locked"),
  };
  assert.throws(() => verify(ci, publish, broken), `an unpinned wasm-pack in ${name} was not detected`);
  const floating = {
    ...benchmarkWorkflows,
    [name]: benchmarkWorkflows[name].replace(new RegExp(`toolchain: ${RUST_RELEASE.replaceAll(".", "\\.")}`), "toolchain: stable"),
  };
  assert.throws(() => verify(ci, publish, floating), `a floating Rust toolchain in ${name} was not detected`);
}
console.log("release gate policy and failure simulations OK");
