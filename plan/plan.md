# Aeris Charts All-in-One Architecture and Competitive Delivery Plan

Aeris will be a complete financial and general visualization library. Lightweight Charts is the
financial competitive reference; Recharts is the general charting competitive reference. This plan
covers the **general (non-financial) charting program** toward full Recharts parity and beyond.
Trading and order-flow work lives in [Expansion.md](Expansion.md); both plans share one
`ChartEngine` and one frame contract.

How to read this file:

1. **Status at a glance** — where every phase and batch stands today.
2. **How work is delivered** — the batch, gate and commit rules.
3. **Delivered work** — what is implemented, with evidence.
4. **Known gaps** — source-confirmed problems and the phase that fixes each.
5. **Phases R0–R8** — remaining work as checklists, with exit criteria.
6. **Parity coverage matrix** — the capability rows that define parity.
7. **Scope, architecture rules, verification and completion** — the standing rules.

Keep this file current: when a batch lands, tick its checklist items, add it to **Delivered work**
with its commit, and update the status table in the same commit. A phase or matrix row changes to
**Verified** only with the evidence required in **Verification and evidence policy**.

## Status at a glance

Updated 2026-09-25. Plan baseline dated 2026-09-23.

> **Paused 2026-09-25.** General-chart work is paused after the R3 range-bar batch (`3fe3b22`,
> recorded in `e063e9c`) so trading and order-flow work in [Expansion.md](Expansion.md) proceeds
> first. Do not start new G1–G8 batches during the pause. Fixes to delivered general-chart behavior,
> and shared work required by Expansion.md (cross-chart sync PD5 and image export PD6, which R4
> later extends), remain allowed. Resume with G1 (R0 and R1) when the maintainer lifts the pause,
> because the trading journal will need the general charts, then continue in dependency order.

| Batch | Phase | Scope | Status | Done so far | Next |
| --- | --- | --- | --- | --- | --- |
| G1 | R0 + R1 | Competitive baseline, lifecycle and mutable object foundations | **Open (paused)** | — | Resume point: pin Recharts version, map the matrix, standalone creation, in-place mutations, failure cleanup |
| G2 | R2 | Scales, axes and responsive layout | **Open** | — | Temporal ticks and views, grid and zero lines, multiple axes |
| G3 | R3 | Cartesian visual and data semantics (remainder) | **In progress (paused)** | 9 delivered R3 slices (see **Delivered work**) | Bars and stacks, gradients, error bars, composition, per-item styling |
| G4 | R4 | Components and interaction | **Open** | — | Legend, tooltip, brush, selection, sync (extends Expansion.md PD5), export (PD6) |
| G5 | R5 | React and framework-neutral authoring | **Open** | — | Composable components over complete mutations |
| G6 | R6 | Polar families and transitions | **Open** | — | Polar transforms, pie/donut, radar, radial bar, polar area, animation |
| G7 | R7 | Hierarchy and flow families | **Open** | — | Funnel, treemap, Sankey, sunburst |
| G8 | R8 | Parity closure and release readiness | **Open** | — | Full matrix verification against the pinned competitor |

Notes:

- R3 slices were delivered ahead of R0–R2. They count toward R3 only; R3 closes after R1–R2 land
  and its exit criteria pass.
- No phase and no coverage-matrix row is verified yet. No competitive parity or release-completion
  claim should be inferred from delivered slices.

## How work is delivered

Work proceeds in **large batches**, as defined in **Work cadence** in [AGENTS.md](../AGENTS.md).
Each batch is one row of the status table: a whole phase (R0 and R1 ship together as G1), not one
option or one series family at a time. Delivered items 1–9 were single-option slices; that cadence
is retired.

- **Implement the whole batch first.** Build every checklist item in the phase, with its regression
  tests and fixtures written as each item is built. Do not stop between items for full gates,
  commits or pushes.
- **Focused checks while implementing.** `cargo check`, unit tests and `cargo clippy` for touched
  crates, and frame fixtures for the affected families. Nothing broader.
- **One full gate at the end.** Run the complete gates in **Verification and evidence policy** once,
  plus Playwright when the batch changes browser-facing behavior and GPUI parity/replay when it
  changes GPUI execution. Fix every failure and rerun until green. If the cause is unclear, rerun
  focused checks item by item.
- **One commit and push per batch.** Commit with a structured message listing delivered checklist
  items and verification, then push `main`. Never commit a batch with a failing or skipped required
  gate.
- **Update this file in the same commit.** Tick the checklist, update **Delivered work** and the
  status table, and update `docs/Architecture.md` when ownership or execution paths changed.
- **Manual evidence at phase closure, not per batch.** Screenshots, accessibility review,
  competitor comparisons and recorded benchmarks are collected once, when the phase closes.

A batch may be split into two commits only when it is too large to review as one, and each part
must pass the full gate on its own.

## Delivered work

All items below are implemented and pushed to `github/main`. They belong to R3.

| # | Capability | Commit | Evidence |
| --- | --- | --- | --- |
| 1 | General-series `line_width` configurable through engine, WASM, TypeScript, persistence and public browser API | `d17a39f` | Slice gates below |
| 2 | General path `line_style`: portable `solid`, `dotted`, `dashed` through the shared frame contract | `819bcbb` | Slice gates below |
| 3 | `xy_area` explicit finite `baseline_value` with shared fill geometry, hit-testing, persistence, WASM/TypeScript and browser coverage | `6a5684b` | Slice gates below |
| 4 | Opt-in `point_markers` with configurable radius on line, area, stacked-area and range-area; shared geometry, matching hit targets, atomic live mutation, V2 persistence, WASM/TypeScript, browser coverage. Also repairs live mutation of items 1–3 | — | Slice gates below |
| 5 | Persisted marker symbols `circle`, `square`, `diamond`, `triangle` for scatter and path markers, with symbol-matched exact hit geometry. Bubble marks remain area-scaled circles | — | Slice gates below |
| 6 | Persisted `linear`, `step`, `curved` interpolation on line, area, stacked-area and range-area; shared path and coupled-band expansion on every renderer with matching exact/nearest hits; stacked areas reject mixed interpolation | — | Slice gates below |
| 7 | Opt-in persisted `connect_missing` on the same families. Missing rows stay queryable but no longer split runs; transform-invalid coordinates remain hard gaps; stacked members share one policy | — | Slice gates below |
| 8 | Bounded persisted `fill_opacity` on area and range-area fills (ordinary, stacked, coupled-band), preserving the gradient relationship; Rust persistence, WASM serialization, TypeScript options, Chromium/Firefox/WebKit round-trip coverage | `1db0dfe` | Workspace tests, workspace and WASM clippy, package lint/build/typecheck/smoke test, general-chart browser matrix (88 passed, 2 skipped) |
| 9 | Category `range_bar` series with low/high bounds, shared rectangle geometry and exact hits, typed/object browser ingestion, persistence-compatible kind mapping, accessibility text, and browser/native regression coverage | `3fe3b22` | Engine tests (including logarithmic Y geometry), workspace tests/clippy/WASM clippy, package lint/build/typecheck/pack smoke, native release perf gate, and the full Chromium/Firefox/WebKit browser suite (363 passed, 15 skipped) |

Slice gates for delivered R3 items (historical; remaining work uses batch gates): applicable Rust tests, clippy (including the WASM target), package
lint/build/typecheck/package smoke test, Chromium browser tests, formatting checks and the native
release performance gate.

These slices do not close R3. The phase still requires the rest of the Cartesian visual and data
matrix and its complete acceptance evidence.

## Known gaps

Source-confirmed gaps and review risks from the 2026-09-23 review. Each is owned by a phase. The
earlier Phase 1–3 completion statements described narrower slices and are superseded: Cartesian
completeness and the all-in-one authoring experience are **open**. Earlier test counts and timings
are historical observations in Git history, not proof of current completion. Existing code and tests
remain valuable and must be preserved.

| Gap | Evidence | Required correction | Phase |
| --- | --- | --- | --- |
| Standalone creation | `examples/web_demo/general_dashboard.js` creates a general pane then removes pane 0 | Explicit initial general domain through the canonical constructor, without a transient financial pane or host cleanup recipe; financial default stays compatible | R1 |
| Mutable public objects | `packages/charts/src/types.ts::general_series_api` and `general_axis_api` | Handles lack option mutation; Rust has visibility mutation but the browser handle lacks it. Add atomic mutations that preserve identity and invalidate affected state | R1 |
| Failed React installation | `GeneralPane` creates a handle, calls `setData`, then records ownership | A failed initial data install can leave an untracked series. Add rollback and failure-path lifecycle tests; review callback exceptions and cleanup ordering | R1, R5 |
| Temporal axes | `general_axes.rs::axis_ticks`, `tick_labels_for_domain`, `pan_general_axis`, `zoom_general_axis` | Temporal data and geometry exist, but temporal ticks fall through to empty output and pan/zoom reject nonnumeric domains. Complete the temporal coordinate contract | R2 |
| Grid and zero lines | `GeneralAxis` stores policies; `persistence.rs` serializes them | Accepted options have no general grid execution. Implement shared frame output and observable toggle tests | R2 |
| Visual configuration | `GeneralSeriesOptions`, `frame/general_series_geometry.rs` | Surface is narrow. Audit and implement documented styles and geometry choices end to end (the delivered slices above began this) | R3 |
| Shared components | Legend, shared-tooltip, brush and reference snapshots in `general_series.rs` | Snapshots alone do not establish a complete interactive legend, tooltip, brush or export experience | R4 |
| React reconciliation | `packages/charts/src/react.ts::GeneralPane` | Changed series options recreate series; changed axes recreate dependent series; configuration arrays instead of component composition. Complete engine mutation and declarative authoring | R5 |
| Chart breadth | `GeneralSeriesKind` has Cartesian variants only | Polar and hierarchy/flow families are open. Funnel, treemap, Sankey and sunburst are in the competitive target, not an indefinite backlog | R6, R7 |
| Documentation | Prior plan examples and `Architecture.md` | Prior scatter example omitted required axis bindings. Architecture places engine-owned general behavior under the core heading and overgeneralizes retained React updates. Correct wording without presenting future code as current | R0 |
| API docs mix | [General_charts_api.md](../docs/General_charts_api.md) | Mixes implemented contracts with proposals | R0 |

These are not an exhaustive defect audit. Coverage-matrix rows are required coverage to audit, not
assertions that every listed feature is absent.

## Phases

All phases are open unless marked otherwise in **Status at a glance**. Existing implementation
counts toward a phase only after its required behavior is demonstrated. Each phase is delivered as
one batch (see **How work is delivered**); its last checklist item is the batch gate, commit and
push. Do not delay fixes until a large framework rewrite, and do not skip foundation work to add a
demo chart.

### R0 — Auditable competitive baseline

**Batch:** G1, together with R1. **Depends on:** nothing. **Status:** open, paused.

- [ ] Pin the Recharts release and source revision; retain the existing financial competitor
      baseline.
- [ ] Inventory public props and components; map every relevant capability to the coverage matrix
      with supported combinations.
- [ ] Add reference fixtures for each matrix row and record intentional differences. Executable
      fixtures stay in existing test infrastructure; transient screenshots and reports stay out of
      committed documentation.
- [ ] Reconcile `General_charts_api.md`, `Public_api.md`, examples and architecture claims against
      exports, manifests, scripts and actual call paths. Separate supported, experimental and
      proposed behavior.
- [ ] Capture clean release financial, general and combined baselines and the current enforced
      budgets.

**Exit:** every required capability has a scoped owner, dependency, fixture and honest status; no
unsupported example or historical PASS substitutes for current evidence. Version, date and evidence
paths accompany future status changes. Unverified rows stay open.

### R1 — Lifecycle and mutable object foundations

**Batch:** G1, together with R0. **Depends on:** R0. **Status:** open.

- [ ] Initial general-domain creation through the canonical constructor.
- [ ] Last-pane ownership defined together with adapter ownership.
- [ ] Atomic in-place axis and series mutations through all public boundaries.
- [ ] Failed installation and cleanup fixed.
- [ ] Visibility and ordering affect domains, legends, hits and exports consistently.
- [ ] G1 full gate green (R0 and R1 items); batch committed and pushed.

**Exit:** standalone general and mixed charts can create, update, rebind, reorder, hide, remove and
restore through actual browser and native paths. Invalid operations leave prior state intact.
Handles, explicit row focus and selection, and unaffected views survive routine changes; repeated
mount and dispose release resources.

### R2 — Scale, axis and responsive layout contracts

**Batch:** G2. **Depends on:** R1. **Status:** open.

- [ ] Temporal ticks, formatting and view operations.
- [ ] Category behavior, including zoom/pan and duplicate labels.
- [ ] Explicit ticks and formatters.
- [ ] Numeric extremes and degenerate domains.
- [ ] Grid and zero lines executed in the frame.
- [ ] Domain padding and clipping.
- [ ] Multiple axes.
- [ ] Axis titles.
- [ ] Small-container behavior.
- [ ] Every reviewed silent-option gap corrected.
- [ ] G2 full gate green; batch committed and pushed.

**Exit:** deterministic domain and coordinate round trips and frame fixtures cover all supported
scale and orientation combinations. Browser resize, font, DPR, pointer and keyboard view tests agree
with native and GPUI output; no accepted option silently does nothing, and layout work has an
enforced bound.

### R3 — Cartesian visual and data semantics

**Batch:** G3 (all remaining items in one batch). **Depends on:** R1–R2. **Status:** in progress,
paused (see **Status at a glance**).

- [x] Line width (item 1).
- [x] Line dash styles (item 2).
- [x] Area baseline value (item 3).
- [x] Point markers and marker symbols (items 4–5).
- [x] Linear, step and curved interpolation (item 6).
- [x] Missing-value connection policy (item 7).
- [x] Area fill opacity (item 8).
- [ ] Remaining curve, gap and baseline policies across every applicable family.
- [ ] Bars and stacks: both orientations, groups, sizing and gaps, corners, mixed signs, stack order
      and required offset modes.
- [x] Range bars (category-band low/high rectangles and exact hits).
- [ ] Error bars.
- [ ] Gradients.
- [ ] Per-item customization (per-point and per-bar styles).
- [ ] Composition of families on shared axes.
- [ ] Audit existing box plot, heatmap and bubble behavior rather than rewriting completed storage
      and geometry.
- [ ] G3 full gate green; batch committed and pushed.

**Exit:** each family passes object and typed ingestion, atomic updates, missing, duplicate and
extreme data, visibility and stack changes, exact and nearest hits, labels, accessibility,
persistence and executor parity. Reference examples demonstrate visual configurability; high-density
fixtures demonstrate bounded work.

### R4 — Chart components and interaction

**Batch:** G4. **Depends on:** R1–R3. **Status:** open.

- [ ] Usable default legends and tooltips.
- [ ] Titles and labels.
- [ ] References.
- [ ] Keyboard and touch brush controls.
- [ ] Selection.
- [ ] Linked-chart synchronization, extending the shared contract delivered first for financial
      charts as [Expansion.md](Expansion.md) PD5.
- [ ] Frame image export, extending the shared contract delivered first as Expansion.md PD6.
- [ ] Localization, overflow and focus behavior.
- [ ] G4 full gate green; batch committed and pushed.

R4 extends the PD5 and PD6 contracts to general domains and chrome; it does not build a second
synchronization or export path.

**Exit:** consumers build an interactive dashboard from published APIs without demo-owned semantic
logic. Legend toggles preserve identity, brushing survives resize, synchronization handles unequal
datasets without loops, and all controls are keyboard and screen-reader usable with bounded
snapshots.

### R5 — React and framework-neutral authoring

**Batch:** G5. **Depends on:** R1–R4. **Status:** open.

- [ ] Composable components and typed data mapping over canonical handles.
- [ ] Controlled and uncontrolled behavior where applicable, events and documented defaults.
- [ ] Migration recipes from Recharts.
- [ ] Equivalent imperative composition for hosts that do not use React.
- [ ] G5 full gate green; batch committed and pushed.

**Exit:** packed-consumer examples cover standalone, composed, synchronized and financial/general
charts. Prop changes retain engine identities; failure, Strict Mode and concurrent lifecycle tests
pass. SSR import, hydration setup, bundler and WASM asset resolution, and cleanup work without
repository paths.

### R6 — Polar families and shared transitions

**Batch:** G6. **Depends on:** R2–R5. **Status:** open.

- [ ] Angular and radial transforms with shared sector and polygon geometry.
- [ ] Pie and donut.
- [ ] Radar.
- [ ] Radial bar.
- [ ] Polar area.
- [ ] Shared general transitions for every family, including existing Cartesian families, with
      interruption and reduced-motion behavior.
- [ ] G6 full gate green; batch committed and pushed.

**Exit:** all polar variants cover degenerate, zero and missing data, angles and radii, label
collision, legends, selection, keyboard navigation, persistence and every executor. Fixed-clock
transition fixtures prove repeatability, correct interaction targets, bounded memory and zero idle
animation scheduling.

### R7 — Hierarchy and flow families

**Batch:** G7. **Depends on:** the lifecycle, layout and primitive contracts above. **Status:** open.

- [ ] Funnel.
- [ ] Treemap.
- [ ] Sankey.
- [ ] Sunburst, using shared polar geometry where appropriate.
- [ ] G7 full gate green; batch committed and pushed.

Each is a dedicated engine layout family with typed validated inputs; do not reuse incompatible XY
storage merely to avoid a proper owner.

**Exit:** each family has deterministic ordering and layout, declared cycle, depth and size policies,
bounded work, update and transition behavior, labels and styles, exact hits, tooltip and legend
behavior where applicable, accessibility, persistence and cross-backend fixtures. These families are
required for competitive closure.

### R8 — Parity closure and release readiness

**Batch:** G8. **Depends on:** R0–R7. **Status:** open.

- [ ] Run the complete matrix against the pinned competitor.
- [ ] Demonstrate financial-only, general-only and combined workloads.
- [ ] Finish documentation and customization/migration examples.
- [ ] Export, backend fallback and device-recovery evidence.
- [ ] Clean-install evidence.
- [ ] Recheck upstream scope; record and assess new upstream features explicitly.
- [ ] G8 full gate green; release evidence committed and pushed.

**Exit:** every required matrix row is verified or has a maintainer-approved, clearly documented
semantic alternative that satisfies the user task. No required family remains demand-deferred.
Publish capability claims only for verified behavior, with measured startup, size, frame, input and
memory results and known limits.

## Parity coverage matrix

Each row must acquire exact versioned reference examples, Aeris API mappings, named automated
fixtures, manual checks where needed, and recorded differences before it can be marked verified.
Status is deliberately conservative: **Partial** means code exists but the full row is unverified;
**Open** means the product contract still needs delivery. These are Aeris requirements, including
platform capabilities beyond the competitor's browser rendering model.

| Capability | Status | Required outcome | Phase |
| --- | --- | --- | --- |
| Standalone and composed charts | Partial | General-only, financial-only and mixed panes; compatible overlays, explicit axes, deterministic ordering and lifecycle | R1, R3 |
| Line, area, range and scatter/bubble | Partial (items 1–8 delivered; range bars are tracked below) | Linear/step/curved interpolation, gap/connection policy, baselines, symbols, active marks, fills/strokes and error bounds | R3 |
| Bars and stacks | Partial (range bars delivered) | Both orientations, groups, sizing/gaps, corners, per-item styling, mixed signs, range bars, stack order and required offset modes | R3 |
| Box plots and heatmaps | Partial | Preserve existing extra families; complete color domains, legends, missing values and interaction | R3 |
| Scales and axes | Partial | Numeric/log/symlog, temporal, category/point, reversed/multiple axes, explicit/auto domains, ticks, formatting, overflow and grid policy | R2 |
| Pie/donut, radar, radial bar, polar area | Open | Polar layout, start/end angles, inner/outer radii, padding, labels/leaders, angular/radial axes and interactions | R6 |
| Funnel, treemap, Sankey, sunburst | Open | Purpose-built deterministic bounded layouts, data contracts, styling, labels, hits, accessibility and updates | R7 |
| Legend and tooltip | Partial | Default usable components, visibility controls, item/shared modes, placement, formatting, ordering, custom content and touch/keyboard behavior | R4 |
| References, labels, titles and grids | Partial | Engine layout and domain contribution, overlap/overflow policy, background/foreground order, style and export consistency | R2, R4 |
| Brush, selection and synchronization | Partial | Pointer/touch/keyboard controls, semantic range handles, domain-aware pan/zoom, linked charts and feedback-loop prevention | R4 |
| Responsive layout | Partial | Zero-size/hidden/revealed containers, constrained/aspect sizing, DPR/font changes, small plots and bounded layout convergence | R1, R2 |
| React authoring | Partial | Composable axes/series/components, typed data mapping, controlled updates, stable identities, Strict Mode, failure cleanup and SSR-safe import | R5 |
| Customization | Partial | Per-item styles, symbols, gradients, dash patterns, label/tooltip formatting, bounded custom marks and explicit host-only content boundaries | R3–R5 |
| Animation | Open for general transitions | Enter/update/exit and interruption with stable identities, bounded retained state, shared timing semantics and reduced motion | R6 |
| Accessibility/localization | Partial | Keyboard operation of every family/control, meaningful bounded snapshots, focus retention, announcements, contrast, locale and text measurement | Every phase |
| Persistence, export and recovery | Partial | Complete schema coverage, atomic restore, callback reattachment, equivalent frame exports, backend failover and clean disposal | Every phase, R8 |
| Packaging and migration | Partial | Packed-consumer examples, framework-neutral and React guides, discoverable API, reproducible competitor comparisons | R0, R5, R8 |

## Scope and parity rules

A basic working dashboard, a list of rendered chart types, or a thin React wrapper does not meet
this goal. Competitive quality includes authoring, visual control, interaction, accessibility,
lifecycle, responsive layout, documentation, distribution and measured performance.

Keep one public library, one `ChartEngine` and one ordered frame contract. Preserve the specialized
financial data and coordinate paths while completing general charting as a first-class capability.
The browser package is `@aeristerminal/aeris-charts` with an optional `/react` entry. Package naming
or distribution changes require a compatibility decision; this plan does not introduce another
product. [Architecture.md](../docs/Architecture.md) describes current ownership and execution; this document
specifies the target and acceptance gates.

The existing shared-engine direction is sound. Replacing it with a browser-only renderer or making
financial storage universally generic would harm Aeris: charts would diverge across backends or
financial updates would pay unnecessary work. Retain the working foundations and finish their
contracts.

**Parity definition.** Parity means equivalent user capabilities, predictable behavior and polished
results. It does not require copying React/SVG internals, identical method names, undocumented
quirks, or arbitrary DOM execution inside Rust. Record deliberate semantic differences and
demonstrate the migration path. An omission cannot be renamed a difference merely to close a
milestone.

**Out of scope for this baseline.** Gauge, arbitrary graph/network, geographic and 3D visualization.
They may be added later without postponing any required row. Full competitiveness is a release gate,
not a promise to implement every conceivable visualization.

**References.** Reviewed on 2026-09-23. R0 must pin the exact released Recharts version or source
revision used by executable comparisons; a moving documentation site is insufficient as a permanent
test baseline.

- [Recharts API catalog](https://recharts.github.io/en-US/api/) establishes Cartesian, polar, composed,
  funnel, treemap, Sankey and sunburst families plus shared components and synchronization.
- [Line API](https://recharts.github.io/en-US/api/Line/) supplies reference behavior for data mapping,
  dots, missing-point connections, styling and animation.
- [XAxis API](https://recharts.github.io/en-US/api/XAxis/) supplies axis/domain/tick configuration coverage.
- [Tooltip API](https://recharts.github.io/en-US/api/Tooltip/) supplies tooltip presentation and behavior coverage.
- [ResponsiveContainer API](https://recharts.github.io/en-US/api/ResponsiveContainer/) supplies sizing coverage.
- [FunnelChart API](https://recharts.github.io/en-US/api/FunnelChart/) documents stacking offsets and synchronization.
- [Sankey API](https://recharts.github.io/en-US/api/Sankey/),
  [Treemap API](https://recharts.github.io/api/Treemap/) and
  [Sunburst API](https://recharts.github.io/en-US/api/SunburstChart/) establish distinct flow/hierarchy contracts.
- [Accessibility guidance](https://github.com/recharts/recharts/wiki/Recharts-and-accessibility)
  informs keyboard and screen-reader comparisons.

## Architecture rules

```text
Framework-neutral API / React authoring / native host
    -> validated commands and bulk data normalization
    -> one ChartEngine
         financial: DataLayer, time union, price/time scales, financial interactions
         general: typed datasets, explicit domains/axes, series and layout families
    -> shared layout, semantic interaction snapshots and ordered ChartFrame
    -> DrawList
    -> Canvas2D | WebGPU | GPUI | native
```

### Ownership and dependency direction

| Owner | Responsibility |
| --- | --- |
| `aeris_charts_core` | Platform-free scale math, validation fundamentals, financial storage and shared option/value types; f64 media-space math |
| `aeris_charts_engine` | General datasets, axes, domain resolution, series/layout algorithms, mutations, interaction, transitions, persistence and frame construction |
| `aeris_charts_render` | Ordered primitives and shared lowering/tessellation math; no host or chart-family policy |
| Executors | Execute prepared primitives with equivalent clipping/blending/text; own bounded device/font/image resources and recovery |
| WASM and TypeScript | Bulk conversion, platform input, resource initialization, typed handles, host callbacks, DOM presentation and accessibility |
| React | Declarative ownership and reconciliation through the public imperative API; no duplicate data/geometry/interaction model |

Do not add crates, generic scene graphs, plugin registries, trait layers or speculative feature flags
just to accommodate the roadmap. Extract cohesive internal modules when their actual responsibilities
justify it. Do not turn growing `general_series.rs` into a universal layout abstraction: hierarchy
and flow need appropriate typed input and algorithms, while sharing lifecycle and frame output.

### Financial isolation and first-class general creation

Keep financial time union, compact OHLC/scalar columns, LOD, indicators, drawings, trading, price
scales and streaming updates authoritative. General work must add no per-row dispatch to those loops
and retain zero general dataset/cache capacity in a financial-only chart. Protect existing public
APIs, V1 restore, input behavior, whitespace and financial golden fixtures.

Add an explicit creation-time domain/topology contract at the engine owner and expose it consistently
through WASM, TypeScript and React. Defaults remain financial. General-only charts must reserve only
the chrome they use. Stable pane IDs must survive reorder; disposal must not require callers to seed
a temporary financial keeper pane. Define the engine's last-pane invariant and adapter ownership
together.

### Data, identity and atomic mutations

Retain typed general columns, explicit validity and stable row IDs. Distinguish financial UTC
seconds, continuous epoch milliseconds, category identity and display text without unit guessing.
Specify ordering, duplicate-X/category policy, missing values and generated-versus-explicit identity
per family.

Provide in-place axis/series option updates and visibility, order and compatible binding changes.
Validate dependent datasets, stacks, references and axes before mutation. Invalid updates must
preserve the entire prior state. A style change retains data, handles, focus/selection and runtime
view unless its documented semantics require otherwise. Structural domain/type changes must be
explicit.

Chart-level object data and React data keys normalize once per changed input; typed streaming
remains a bulk path. Decide shared-column ownership from real composed-chart callers and measurements
before adding public dataset machinery. Host accessors never run inside frame or hit-test loops.
Hierarchy and flow input require stable node/link identities, validation of references/cycles as
applicable, defined ordering and depth/size/work caps; they must not be forced into XY rows.

### Layout, scales and shared geometry

Resolve data domains and runtime views separately. The same transform must drive ticks, grids,
geometry, hits, brushes, references and accessibility. Complete temporal interval selection and
formatting with an explicit deterministic timezone policy; never silently use the browser timezone.
Category zoom/pan and duplicate labels need declared semantics. Test extreme and degenerate domains.

Layout reserves chart content, titles, legends, axis strips and plot regions through bounded passes.
Define behavior when text/axes do not fit rather than letting clipping or layout oscillation decide.
Font, DPR, formatter and locale changes invalidate measurements. Host measurement is permitted;
host-owned tick selection or autoscale is not.

General grids use resolved general ticks in the ordered background layer. Reference/background
fills, series, interaction chrome and labels need explicit order and clips. New curves, sectors,
polygons, symbols, rounded shapes or gradients must have shared geometry/lowering and every executor
implemented before their public feature is complete. Retain f64 until the documented encoding
boundary.

### Components, interactions and extension boundaries

The engine owns component meaning, content snapshots, anchors, selections and reserved plot space.
Hosts may present HTML tooltips, semantic controls or accessible DOM. Default legends/tooltips must
be usable without copying demo code; optional rich HTML does not become the only implementation of
built-in chart geometry. Define what exports include and provide frame-rendered equivalents for
built-in chrome. Document host-only custom content limitations.

Unify pointer, touch and keyboard commands for general selection, brushing and view changes. Derive
axis-versus-item tooltip membership from the correct oriented domain, including horizontal bars,
duplicate values, missing rows and mixed series. Synchronization uses semantic values or declared
index matching with explicit mismatch policy. A bounded host coordinator may route events between
independent charts; each receiving engine resolves its own semantics. Source/revision tracking must
prevent loops and disposal must remove subscriptions.

Customization receives bounded read-only snapshots or returns validated styles/marks through an
explicit host boundary. Do not promise portable execution of arbitrary SVG/React elements. Built-ins
must remain available on native and headless paths, with clear migration equivalents for common
Recharts customization tasks. Review licenses before copying any external code or assets.

### React and transitions

Build declarative axes, Cartesian/polar series, legend, tooltip, labels, references and brush over
complete imperative mutations. Retain GeneralPane compatibility. Stable keys retain identities;
ordinary options/visibility/data changes do not recreate chart objects. Validate a reconciliation
batch before destructive operations and roll back newly acquired resources on failure. Test changed
kinds, invalid data, callback failures, parent/child cleanup, async initialization, stale closures,
concurrent rerenders and Strict Mode. SSR-safe import and static server-rendered chart output are
different capabilities; document each accurately.

General transitions are engine-owned interpolation sampled with an explicit monotonic clock supplied
by hosts. Bound duration, retained prior geometry and active transitions; interrupt from the current
presentation, reconcile hit/focus behavior, and stop scheduling at rest. Reduced motion disables or
shortens transitions deterministically. Streaming financial updates retain their established policy.

### Invalidation, performance and persistence

Classify mutations by data, domain, layout, geometry, paint and interaction impact. Record generation
inputs for retained state; remove caches with their owner and rebuild device resources after loss.
A row cap alone does not prove bounded interactive latency. Measure worst-case dense/overlapping
hits, stack alignment, category unions, long labels, many axes/series and hierarchy depth. Reuse
existing LOD/index mechanisms where appropriate; add optimizations only after release measurements.

Persist new semantic configuration with versioned migrations and transactional restore. Preserve V1
financial compatibility and current V2 contracts; version schema extensions when compatibility
requires it. Runtime callbacks, DOM, device resources and transient animation/hover/focus are not
serialized. Restored live handles must not alias stale handles. State exports and image exports are
separate gates.

## Verification and evidence policy

Each implementation starts with a failing regression or measurable invariant through its actual
public host/executor path. Shared-engine unit tests alone cannot close a browser or rendering
requirement.

Verification cadence:

- **During a batch:** focused checks only (touched-crate check, tests and clippy, and the affected
  families' frame fixtures).
- **End of batch:** the complete gates below once, plus Playwright and GPUI parity/replay when the
  batch affects those paths; then commit and push the batch.
- **Phase closure:** coverage-matrix verification, manual screenshots, accessibility review,
  competitor comparisons and recorded benchmarks.

Run the complete gates required by [AGENTS.md](../AGENTS.md) at the end of each batch, before
committing code:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p aeris_charts_wasm --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
AERIS_CHARTS_PERF_STRICT=1 cargo run -p aeris_charts_native --example perf_gate --release

cd packages/charts
npm ci
npm run lint
npm run build
npm run typecheck
npm run test:pack
```

Also run public API/namespace/release-policy guards, applicable Chromium/Firefox/WebKit Playwright
coverage, native golden/frame checks, and GPUI parity/replay checks for affected execution. Keep
portable correctness blocking and calibrated machine-specific visual/performance evidence labeled.
Run performance thresholds in strict mode as CI does; never relax tests or budgets to conceal
regressions.

Use `benchmarks/benchmark.mjs` and `benchmarks/budgets.json` for versioned release evidence. Current
policy v3 includes a 2,000 ms general-dashboard startup ceiling and 100,663,296-byte first-frame
upload ceiling, alongside package-size limits. These limits are existing guards, not a declaration
that their ceilings are competitive targets. Preserve them until measured evidence supports an
explicit revision. Add family-specific budgets before closure, including p95 input/frame latency,
steady-state allocation, retained CPU/GPU memory, upload work, cold startup and repeated disposal.
Compare equal data, viewport, DPR, interactions and release builds; disclose hardware/browser/font
versions and unsupported metrics.

A matrix entry can be marked **verified** only with a commit/revision, exact fixture commands and
results, backend/browser coverage, relevant performance evidence, and recorded manual checks.
Screenshots must cover small and large containers, light/dark themes, long/Unicode labels, overflow
and active states. Accessibility requires interaction and assistive-technology review, not just
snapshot existence.

Documentation-only revisions may skip runtime gates. Check diffs, links/paths, source consistency and
documentation hygiene. This plan revision makes no production ownership or execution change; update
`Architecture.md` in the same commit as future code that changes those contracts, and correct current
wording discrepancies during R0. Preserve unrelated working-tree changes and stage only task-owned
files.

## Definition of completion

Aeris is competitively complete for this plan when a consumer can build the full required Recharts
capability matrix through a coherent published API, combine it with the established financial
product, and rely on equivalent semantic output across supported backends. Routine changes retain
identity; invalid updates are atomic; controls are accessible; styles and layouts are deliberate;
resources and work are bounded; installation and migration are documented; and all release gates have
current evidence.

Until then, report delivered items and remaining gaps precisely against the phase checklists and
matrix rows above. A working demo or a green subset of tests is progress, not completion of the
all-in-one library.
