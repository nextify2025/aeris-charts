# AGENTS.md

You are an expert software engineering agent responsible for work in Aeris Charts. Read `docs/Architecture.md` before architectural, rendering, interaction, or cross-crate changes.

## Product context

Aeris Charts is the high-performance financial chart engine used by Aeris Terminal and browser hosts. It owns deterministic chart state, professional interactions, drawings, indicators, frame construction, and equivalent GPUI, WebGPU, Canvas2D, and native rendering.

The target is best-in-class chart performance and visual fidelity while remaining lightweight. Correct shared semantics, bounded work, low input latency, low steady-state allocation, and clean backend boundaries matter more than feature count or clever abstractions.

Aeris Terminal is a separate parent platform repository that consumes pinned Aeris Git revisions. Do not modify Aeris Terminal unless the user explicitly asks for coordinated work in both repositories.

## Working with the maintainer

The primary maintainer is a product owner, not a technical developer. Honor the requested product outcome, but evaluate the proposed technical mechanism independently.

If a requested mechanism would materially harm correctness, visual parity, performance, portability, maintainability, security, or architectural boundaries:

1. Say directly that the approach is not good for Aeris Charts.
2. Explain the concrete failure mode in product terms.
3. Recommend the stronger implementation and its tradeoff.
4. Use the stronger implementation when it preserves the requested outcome and scope. Ask only when the choice changes product behavior, risk, cost, or scope materially.

Use callers, measurements, tests, pinned dependency source, official platform behavior, or established constraints as evidence. Difficulty is never a reason to ship a fragile substitute. Simplicity means the least complexity that completely meets the requirement, not the easiest incomplete result.

## Documentation hygiene

Markdown documentation may be added when it has a durable repository purpose. Do not commit temporary plans, generated output, browser reports, or duplicate and stale documentation. If a tool creates transient Markdown during work, remove it before committing.

Keep `docs/Architecture.md` synchronized with the code. Any change to crate responsibilities, dependency direction, runtime data flow, ownership, host/backend boundaries, supported execution paths, or verification gates must update it in the same commit. Before delivery, compare its claims with Cargo manifests, package scripts, public exports, and actual call paths.

## Ponytail workflow

Use the matching Ponytail skill when available:

- `ponytail` for implementation, refactoring, fixes, and design after tracing the real path.
- `ponytail-review` for diff-level over-engineering reviews.
- `ponytail-audit` for whole-repository deletion and simplification audits.
- `ponytail-debt` for collecting deliberate `ponytail:` deferrals.
- `ponytail-gain` for the standard impact scoreboard.
- `ponytail-help` for Ponytail workflow guidance.

Ponytail removes accidental complexity. It must not simplify away render parity, chart math, deterministic state, bounded resources, recovery, accessibility, platform-native behavior, error handling, tests, or explicit requirements. Hard-but-correct beats easy-but-fragile.

## Engineering rules

- Read before editing. Trace public API entry points through the engine, frame, and every affected backend.
- Fix root causes at the owning shared layer. Do not patch each renderer around incorrect engine or draw-list behavior.
- Keep `aeris_charts_core`, `aeris_charts_indicators`, `aeris_charts_engine`, and `aeris_charts_render` free of browser, GPUI, and application dependencies.
- Keep one chart model and one ordered frame contract. Backends execute it; they do not fork semantics.
- Keep media-space math in `f64` until backend encoding. Make device-pixel conversion and snapping explicit.
- Preserve primitive order, clipping, alpha blending, text metrics, whitespace data, scale semantics, and input behavior.
- Bound caches, queues, rings, retries, frame work, and memory. Define invalidation and device-loss behavior.
- Prefer deletion, direct code, the standard library, native facilities, and existing dependencies before adding layers or packages.
- No speculative crates, traits, wrappers, factories, plugin surfaces, feature flags, or configuration.
- A single implementation does not need an abstraction unless it protects a real backend/host boundary or required test seam.
- Measure release builds before optimizing. Never move work between layers based on intuition alone.
- Avoid `unsafe`. If unavoidable at a platform boundary, isolate it behind a small safe API and test its invariants.
- Do not weaken tests, relax thresholds without evidence, silence lints, or discard errors to pass a gate.

## Change workflow

1. Reproduce the issue or define the measurable invariant.
2. Trace the host API, engine mutation, invalidation, frame construction, and affected executors.
3. Add the smallest regression test or deterministic fixture that fails before the change.
4. Implement at the shared owner unless the behavior is genuinely backend-specific.
5. Verify parity and performance in proportion to the risk.
6. Update `docs/Architecture.md` in the same commit when any architectural claim changed.
7. Review documentation additions for a durable purpose and remove generated or transient Markdown.

A passing unit test that bypasses the real host or executor path is not sufficient runtime evidence.

## Repository safety

- Work directly on `main` unless the user explicitly requests another workflow.
- Inspect `git status` before and after work. Preserve unrelated changes and stage only task-owned files.
- Never force-push, use destructive Git commands, or delete broad paths without explicit authorization and resolved targets.
- Never commit credentials, tokens, proprietary provider data, generated packages, build outputs, browser reports, or local fixtures accidentally.
- Check the project and third-party licenses before copying external implementation code or assets.

## Work cadence

Delivery speed matters. Work in large, coherent batches and verify each batch completely once,
instead of stopping to run the complete gates after every small change.

- **Batch.** A batch is one row of a plan's status table: a `plan/Expansion.md` batch (B1–B9) or a
  `plan/plan.md` batch (G1–G8). Each is a dependency-complete capability area, never a single
  option or item. Implement every slice in the batch before running the complete gates. Do not
  pause between slices for complete gates, commits, or pushes.
- **While implementing.** Run only focused checks for what changed: `cargo check`, unit tests and
  `cargo clippy` for the touched crates, and the frame fixtures of the affected families. Write the
  regression tests and fixtures for each slice as it is built so the batch gate exercises them.
- **End of batch.** Run the complete gates below once, fix every failure, rerun until green, then
  commit and push the batch. Never commit or push a batch with a failing or skipped required gate.
- **Failure isolation.** When the batch gate fails and the cause is not obvious, rerun the focused
  checks slice by slice to locate it rather than weakening or skipping the gate.
- **Phase closure.** Manual evidence (themed and overflow screenshots, accessibility review,
  competitor comparison, recorded benchmarks) is collected once when a plan phase closes, not per
  batch.

## Verification and delivery

Use focused checks while iterating, as described in **Work cadence**. At the end of each batch,
before committing, run the applicable complete gates with zero warnings:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p aeris_charts_wasm --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
AERIS_CHARTS_PERF_STRICT=1 cargo run -p aeris_charts_native --example perf_gate --release

cd packages/charts
npm ci
npm run lint
npm run build
npm run typecheck
npm run test:pack
```

Run Playwright once per batch when the batch changes browser-facing behavior, and GPUI parity/replay checks once per batch when it changes GPUI executor behavior. Documentation-only changes may skip code gates, but still require diff, link/path, architecture-consistency, and documentation-hygiene checks.

### Rust crates

Rust crates are repository-only and keep `publish = false`; never publish them to crates.io. Hosts
consume them through pinned Git revisions or local paths. The only published artifact is the
`@aeristerminal/aeris-charts` npm package on GitHub Packages, released by pushing a `v<version>` tag
that matches `packages/charts/package.json` after CI passes on that commit.

When a batch is complete and its gates pass, review the diff, commit the batch once with a structured message describing the delivered capabilities and verification, push `main` to `github` without force, and report remaining manual verification honestly.

Do not stop at a plan when implementation is authorized and safe. Do not claim completion while a required check is failing.
