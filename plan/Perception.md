# Aeris Charts Agent Perception Plan

Aeris Charts will give AI agents the same reading of a chart that a skilled discretionary trader
has: swings, structure, levels, lines, patterns, order-flow behavior and context. Each item will be
expressed as exact, point-in-time, auditable facts rather than pixels or raw numbers. On top of that
perception, an agent can turn a plain-language strategy into a typed rule. The engine then evaluates
the rule deterministically, measures its historical evidence honestly, and shows every decision on
the human's chart.

The program's thesis is one sentence: **the engine that draws the human's chart is the only place
that can tell an agent exactly what the human is looking at, as of any moment, without guessing.**

How to read this file:

1. **Status and decisions**: where the program stands and what the maintainer must decide.
2. **Why this program**: the evidence behind the approach and the gap it fills.
3. **The perception contract**: the core concepts every batch builds on.
4. **Batches P1–P6**: the work, as checklists with exit criteria.
5. **Evaluation program**: how we prove that it works, not just that it runs.
6. **Scope, ownership and architecture rules**.
7. **Risks and non-goals**.

Item IDs (PC, PF, PS, PE, PX) are stable once the plan is accepted. Batches group items; they do not
renumber them.

## Status and decisions

Created 2026-10-02. **Status: proposed. No batch may start until D1–D4 are decided.**

| Batch | Scope | Depends on | Status |
| --- | --- | --- | --- |
| P1 | Perception foundation: fact model, point-in-time semantics, bar vocabulary, multi-scale swings, structure, swing zones, Chart Brief v1, perception drawing layer | Replay (B5), drawing contract (B2) | Proposed |
| P2 | Geometry and patterns: trendlines, channels, clause-scored patterns, triggers, relation graph, confluence, drill-down | P1 | Proposed |
| P3 | Order flow, liquidity and multi-timeframe perception | P1; tape (B3), depth (B6), profiles and resampling (B7) | Proposed |
| P4 | Setup contract: typed strategy rules, validation, live evaluation, events, trading intents | P2 | Proposed |
| P5 | Evidence: point-in-time historical evaluation, outcomes in R, honest statistics, trial ledger, holdout lock, decision snapshots | P4; image export (PD6) | Proposed |
| P6 | Agent benchmark, documentation and program closure | P5 | Proposed |

### Maintainer decisions required

- **D1 — Priority.** Expansion B7–B9 are open. Recommendation: finish B7 (P3 needs its resampling
  and profiles), then run P1–P2 before B8 and B9. Fold Expansion's I3 structure tier (swing
  points, structure breaks, fair value gaps, order blocks, period levels, opening range) into
  P1–P3 so that structure is built once, as perception facts that also render as studies, rather
  than twice.
- **D2 — Typed strategy rules.** Expansion lists "a Pine-style scripting language" as out of scope,
  and says alert conditions on study outputs are host-evaluated. P4 adds a typed, bounded rule
  specification that the engine evaluates. It is data (a validated tree of conditions over named
  facts), not a language: no loops, variables, functions or interpreter. Recommendation: accept
  it and amend those two Expansion lines in the same commit that starts P4. Without engine-side
  evaluation, each host re-implements rule semantics and the discipline guarantee disappears.
- **D3 — Where evidence lives.** Recommendation: single-chart evidence (every occurrence of a setup
  on the retained history of one chart, with outcomes and summary statistics) is engine work,
  because only the engine can guarantee point-in-time correctness. Cross-symbol scans, portfolio
  statistics and trial ledgers spanning many charts belong to Aeris Terminal.
- **D4 — Agent connectivity.** Recommendation: the engine exposes typed Rust, WASM and TypeScript
  APIs plus canonical JSON. The agent-facing tool server (for example an MCP server), model choice,
  prompts and broker execution belong to Aeris Terminal. The engine never calls a model.

When D1–D4 are accepted: set this file's status to active, add the P batches to **Work cadence**
in [AGENTS.md](../AGENTS.md), and record the D1 and D2 amendments in [Expansion.md](Expansion.md).

## Why this program

### What the evidence says

- **Vision models do not read trading charts reliably.** MME-Finance (2024) found that frontier
  multimodal models perform poorly on candlestick and indicator charts. A 2026 benchmark, "Do
  VLMs Truly Read Candlesticks?", found useful prediction only in persistent trends, along with
  strong directional bias and weak temporal precision. An April 2026 practitioner audit tested
  four frontier models on 40 real signals. Direction calls were statistically indistinguishable
  from a coin flip, one of 215 pattern identifications was correct, and stated confidence was
  uncorrelated with correctness. Sending screenshots to an agent is therefore not a strategy.
- **Raw numbers are the wrong shape.** Serializing thousands of OHLCV rows costs on the order of
  100k+ tokens and leaves the model to re-derive swings, levels and patterns in its head. That is
  exactly the step where it hallucinates. The time-series tokenization literature (LLMTime and
  later comparisons) shows the result depends heavily on representation, and no raw encoding is
  universally good.
- **Visual patterns are computable, and some carry information.** Lo, Mamaysky and Wang (2000)
  detected classical patterns algorithmically with kernel regression and found that several add
  incremental information. Jiang, Kelly and Xiu (2023) showed that chart-shaped price information
  predicts returns. Their headline Sharpe ratios are gross, equal-weighted long-short and
  small-cap-heavy, so they are not a promise of tradable profit. Perceptually Important Points
  (Chung and Fu, 2001) give a principled way to compress a series into the few points humans
  notice.
- **Pattern edges decay and fail often.** Bulkowski's long-running statistics show failure rates
  for classical patterns roughly doubling, from about 14% in the 1990s to about 28% in 2003–2007.
  Any honest system must measure each rule's evidence on current data rather than assert it.
- **Most reported LLM trading gains are memorization.** "Profit Mirage" (2025) and several 2026
  papers show that agents recall prices and post-hoc narratives from training data. Their
  backtests look predictive inside the training window and fall to random after it.
  Anonymization ("blindfolded" inputs) is the main proposed defense.
- **Backtests overfit by default.** The Deflated Sharpe Ratio and the Probability of Backtest
  Overfitting (Bailey and López de Prado) quantify how much of a "best" result is selection from
  many trials. Neither works unless every trial is counted.

### What already exists, and the gap

| Category | Examples | What it gives an agent | What it cannot give |
| --- | --- | --- | --- |
| Market-data MCP and APIs | TradingView MCP (public beta, Sept 2026), Alpha Vantage MCP, EODHD, Tradier | Quotes, bars, precomputed indicators, screeners | Any perception: the agent still sees numbers, not structure |
| Automated chart analysis | TrendSpider, TradingView auto patterns, Autochartist | Labels and drawings made for human eyes | Point-in-time guarantees, clause-level evidence, a machine contract, a link to the exact chart a human sees |
| Vision on screenshots | FinVision-style multi-agent frameworks | Images | Precision; benchmarks show near-random reliability |
| Natural language to backtest code | QuantConnect Mia, Composer, BacktestBench agents | Generated Python or strategy code | Correctness by construction: the agent writes the code that grades the agent, and look-ahead bugs are easy to introduce |
| Validation tools | VARRD-style statistics engines | Multiple-testing corrections, out-of-sample locks | Perception; they test rules someone else had to express |

Nobody unifies perception, rules, evidence and the human's chart in one deterministic engine. The
gap is a **perception contract** with these properties:

1. **Point-in-time.** Every fact records when it became knowable. Asking "what did the chart show
   at 10:42?" returns exactly what a trader could have seen at 10:42. This reuses the PD2 replay
   clock, which already masks every series, study, footprint cell and marker.
2. **Same geometry as the human's chart.** A fact is not a description of a drawing; it is the
   drawing's source. What the agent reasons about and what the human audits cannot diverge.
3. **Compressed for reasoning.** A ranked, budgeted Chart Brief with stable IDs gives the agent a
   few thousand tokens of structure instead of a hundred thousand of rows, and the agent can expand
   any fact on demand.
4. **Portable and memorization-resistant.** A normalized frame expresses facts in volatility units
   and bars-ago, with optional anonymization. Rules transfer across symbols, and training-data
   recall cannot substitute for reasoning.
5. **Referee, not author.** The agent expresses a strategy as a typed rule. The engine evaluates
   it, emits signals, and measures historical outcomes with every trial counted. The agent's
   confidence is grounded in measured base rates, never in its own feeling.
6. **Order-flow perception.** Absorption, delta divergence, stacked imbalance at a level, and
   liquidity walls being pulled are things traders only see visually today. Aeris already
   owns the tape, footprint and depth stores (B3, B6), so it can perceive them; few competitors can.

### What this does and does not solve

The user's goal is an agent that trades a defined strategy with perfect discipline. Perception
solves *seeing*. The setup contract solves *following the rule* (no skipped stops, no revenge
trades, no hesitation). Evidence solves *knowing whether the rule deserves trust*. None of these
creates an edge where the rule has none. Many discretionary setups owe part of their results to
judgment that does not survive formalization, and the program must report that honestly rather
than hide it. Aeris Charts provides perception and evidence; trade decisions and execution remain
with the host and its user.

## The perception contract

### PC1 — Fact

A fact is one typed perception record:

- **Identity:** stable `FactId`, kind, scale, the source series or stream, the parameters used, and
  an algorithm version.
- **Geometry:** anchors as logical bar plus full-resolution time plus price, reusing the F1 anchor
  model so that facts survive prepend and rebuild exactly as drawings do.
- **Lifecycle:** `formed_at` (the first bar on which it existed as a candidate), `confirmed_at`
  (the bar on which its defining condition was satisfied), and `ended_at` with an end reason
  (invalidated, completed, expired, superseded). Facts are provisional until confirmed. For
  example, the existing ZigZag emits its current extreme as a provisional endpoint. Perception must
  expose that provisional state explicitly, never as a confirmed swing.
- **Measures:** kind-specific numbers (touch count, reaction size, duration, slope), each in both
  absolute units and the normalized frame (PC4).
- **Clauses:** for compound facts (patterns, setups), each defining condition and whether it held,
  with its measured value and tolerance. There is never a single opaque "confidence".
- **Triggers:** the exact prices or times at which the fact would change state (PC5).
- **Projection:** the drawing it renders as, through the existing drawing contract.

### PC2 — Point-in-time semantics

Every perception query takes an optional as-of clock. With the replay clock set, perception output
must equal the output computed from data truncated at that clock. Detection algorithms may only
use information available at the bar on which they publish. Confirmation delay is part of the
fact, not hidden. This is the property that makes evidence (P5) trustworthy, and it is tested
directly (PX4).

### PC3 — Chart Brief

The brief is a deterministic, budgeted summary of the chart, available as canonical JSON and as a
canonical compact text form:

- Facts are ranked by salience: scale, recency, proximity to current price in volatility units,
  strength, and confluence. The brief is cut to a caller-supplied fact or byte budget.
- Stable IDs let the agent ask follow-up questions (`expand(id)`, `facts_near(price)`,
  `facts_between(t0, t1)`, `as_of(clock)`).
- The text form uses fixed vocabulary, fixed field order and fixed precision, so the same chart
  produces the same text byte-for-byte. Agents and tests can diff it.

Illustrative text form (format to be finalized in P1):

```text
BRIEF v1  as_of=bar 4812  frame=normalized  atr14=1.00u
REGIME  vol=p72 trend.major=up(HH3,HL3) trend.minor=down(LH2,LL1) range.minor=[-1.8u,+0.6u]
Z7  resistance  +0.6u  touches=4 last=9b ago reaction.avg=2.1u role=flip(was support)
Z3  support     -1.8u  touches=3 last=31b ago reaction.avg=3.4u
S41 swing.low   minor  -1.1u  6b ago  confirmed
T5  trendline   rising major  through S12,S27,S38  value.next=-1.4u  touches=3
P2  pattern bull_flag minor  clauses 5/6 (pole=4.2u ok; retrace=38% ok; vol.contract=fail)
    confirm: close>+0.6u   invalid: close<-1.9u   measured.target=+4.8u
REL Z7~P2.breakout (0.0u)  T5~Z3 (0.4u)  CONFLUENCE Z3+T5 at -1.6u..-1.8u
```

### PC4 — Normalized frame and anonymization

- Prices are expressed as distance from the current price in units of a declared volatility
  measure (default ATR(14), host-overridable). Times are expressed as bars-ago, and volume as
  z-scores or percentiles.
- Absolute values remain available alongside. The frame changes the presentation, not the facts.
- Anonymized mode omits symbol, absolute dates and absolute prices from the brief, so the agent
  cannot match the chart to memorized history. The host decides when to use it; the engine
  guarantees that no identifying field leaks in that mode.

### PC5 — Triggers and sensitivities

For each fact, the engine computes the state-change thresholds. Examples: "structure breaks on a
close below X", "the trendline's value at the next bar is Y", "the pattern confirms on a close above
Z". These are exact, engine-owned numbers. Agents set alerts and orders from them instead of
recomputing geometry. They map directly onto existing alert lines and trading objects.

### PC6 — Relations and confluence

Facts form a bounded relation graph: tests (a swing tested a zone), breaks, aligns-with (two facts
within a tolerance in volatility units), part-of (swings inside a pattern), and diverges-from (a
price swing against a delta swing). Confluence, meaning several independent facts agreeing at one
price, is what experienced traders look for. It becomes an explicit, queryable fact rather than an
intuition.

### PC7 — Perception layer

A read-only, engine-owned drawing layer projects facts, setups and occurrences onto the chart
through the existing drawing contract and ordered frame, identically on every executor. Hosts can
toggle it, filter it by kind or scale, and highlight the facts an agent cited in a decision. The
human sees exactly what the agent saw.

## Batches

Work cadence follows [AGENTS.md](../AGENTS.md): implement a whole batch, use focused checks while
building, run the complete gate once, then commit and push once per batch, with
`docs/Architecture.md` updated in the same commit.

### P1 — Perception foundation

**Scope:** PC1–PC4, PC7, PF1–PF4. **Depends on:** B2, B5. **Status:** proposed.

- [ ] **PC1** Fact model with lifecycle, measures, clauses, provenance and drawing projection;
      bounded fact store per bound source with explicit caps, eviction and memory telemetry.
- [ ] **PC2** As-of queries on the replay clock. Incremental tip updates; historical corrections
      rebuild from the nearest checkpoint and report the work done.
- [ ] **PF1 Bar vocabulary.** Per-bar normalized descriptors: body and wick ratios, range
      percentile, gap, close location, volume percentile, inside, outside and engulfing
      relations. Pure functions in `aeris_charts_indicators`.
- [ ] **PF2 Multi-scale swings.** Volatility-scaled pivots at three scales (micro, minor, major)
      with explicit confirmation lag and provisional endpoints, plus Perceptually Important
      Points for shape compression. This builds on the existing ZigZag rather than duplicating
      it.
- [ ] **PF3 Structure.** Per-scale trend state (higher highs and higher lows, or lower highs and
      lower lows), structure breaks and changes of character, ranges and compression boxes, and
      leg statistics (size, duration, slope, overlap, impulse or corrective). Absorbs the I3
      swing and structure-break items.
- [ ] **PF4 Swing zones.** Support and resistance zones clustered from confirmed swing prices,
      with touch count, reaction strength, age, last test and role flips; prior-period highs and
      lows and the opening range from host-supplied boundaries. Absorbs the matching I3 items.
- [ ] **PC3** Chart Brief v1 (JSON and canonical text) with salience ranking and budgets.
- [ ] **PC4** Normalized frame and anonymized mode.
- [ ] **PC7** Perception drawing layer on every executor.
- [ ] Rust, WASM and TypeScript APIs (`chart.perception()`), typed schemas for every parameter,
      and persistence of perception settings (never of derived facts).
- [ ] Fixtures: point-in-time equivalence (PX4), cross-executor parity of the layer, and
      deterministic brief bytes for versioned datasets. `perf_gate` budgets for tip update and
      brief generation.
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** for every fact kind, perception at any replay clock equals perception on truncated data.
Briefs are byte-identical across native and browser builds, and the layer renders identically on
every executor within the existing parity tolerances.

### P2 — Geometry and patterns

**Scope:** PC5, PC6, PF5, PF6. **Depends on:** P1. **Status:** proposed.

- [ ] **PF5 Lines and channels.** Trendlines fitted through three or more confirmed swings within
      a volatility-scaled tolerance, parallel and regression channels, touch quality, and breaks.
- [ ] **PF6 Patterns.** Double and triple tops and bottoms, head and shoulders (regular and
      inverse), ascending, descending and symmetric triangles, wedges, flags and pennants,
      rectangles, and cup with handle. Each is defined as published clauses with tolerances and
      carries a neckline or breakout level, an invalidation level and a measured target. Patterns
      reuse the same geometry as the B8 pattern drawing tools.
- [ ] **PC5** Triggers and sensitivities for every fact kind.
- [ ] **PC6** Relation graph and confluence facts, bounded per chart.
- [ ] Drill-down queries: `expand`, `facts_near`, `facts_between`, `as_of`.
- [ ] Fair value gaps and order blocks as facts (absorbs the remaining I3 items), with documented,
      parameterized and deterministic rules.
- [ ] Perception agreement study started (PX1) on a fixed annotated set.
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** every pattern's clauses are documented and fixture-tested on synthetic charts that pass
and fail each clause. Triggers match the bar on which state actually changes in replay.

### P3 — Order flow, liquidity and multi-timeframe perception

**Scope:** PF7–PF9. **Depends on:** P1, B3, B6, B7. **Status:** proposed.

- [ ] **PF7 Order-flow facts.** Price and cumulative-delta divergence across swings, absorption
      (high volume at a level without price progress), exhaustion, stacked imbalances at zones,
      and large-trade clusters, all derived from the canonical tape. Shares rules with OF13
      rather than duplicating them.
- [ ] **PF8 Liquidity facts.** Persistent resting-liquidity walls, pulled and replenished
      liquidity, and level tests against walls, derived from the B6 depth store with its existing
      bounds. Unknowns stay unknown; nothing infers hidden liquidity.
- [ ] **PF9 Context and multi-timeframe.** Volatility regime percentile, session position from host
      boundaries, profile levels (POC, value area, naked POCs from B7), VWAP and anchored-VWAP
      distance, and higher-timeframe structure and zones via F6 resampling, all aligned without
      look-ahead.
- [ ] Brief sections for order flow, liquidity and context. Every order-flow fact is labeled with
      its data source (tape or candle approximation).
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** order-flow and liquidity facts are point-in-time equivalent in replay. Higher-timeframe
facts never reveal an unfinished higher-timeframe bar's future values.

### P4 — Setup contract

**Scope:** PS1–PS4. **Depends on:** P2, decision D2. **Status:** proposed.

- [ ] **PS1 Setup specification.** A typed, versioned, bounded tree with:
      - entry conditions over facts, measures and relations: and/or/not, comparisons, sequence
        ("A then B within N bars"), and scale and kind filters;
      - invalidation conditions;
      - stop and target placement referencing facts (for example "stop below the zone's lower edge
        by 0.2 volatility units");
      - risk sizing in R, a time stop, and session filters from host boundaries.
      No loops, variables, user functions or code.
- [ ] **PS2 Validation that refuses to guess.** Schema validation reports ambiguous, unsupported or
      contradictory clauses back to the agent by path. A strategy the engine cannot express
      exactly is rejected with reasons, never silently approximated.
- [ ] **PS3 Live evaluation.** Bounded incremental evaluation on each update, emitting setup events
      (armed, triggered, invalidated, stopped, target, timed out). Every event cites the fact IDs
      and clause values behind it.
- [ ] **PS4 Discipline path.** Triggered setups produce trading intents through the existing
      trading-intent path, with stops and targets attached. The host and its user decide whether
      intents are executed. Events and intents project onto the perception layer.
- [ ] Persistence and migration for setup specifications; replay equivalence for events.
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** a fixed suite of plain-language strategies has reference specifications whose events
match hand-verified bars in replay, and unsupported phrasing is rejected with actionable paths.

### P5 — Evidence

**Scope:** PE1–PE4. **Depends on:** P4, PD6, decision D3. **Status:** proposed.

- [ ] **PE1 Historical occurrences.** Evaluate a setup over the retained history with
      point-in-time semantics. List every occurrence with entry, exit and reason, outcome in R,
      maximum favorable and adverse excursion, bars held, and the facts cited. Costs and slippage
      are host inputs, never defaults.
- [ ] **PE2 Honest statistics.** Occurrence count, win rate with a Wilson interval, expectancy in R
      with a confidence interval, profit factor, maximum drawdown in R, and outcome distribution.
      Small samples are flagged rather than summarized as confident. Deflated metrics use the
      trial ledger.
- [ ] **PE3 Trial ledger and holdout lock.** Every specification variant evaluated on a chart is
      counted, so selection bias is measurable. An optional holdout window is evaluated once per
      specification hash and then locked.
- [ ] **PE4 Decision snapshots.** Each setup event can render a "what the agent saw" image through
      PD6, with the perception layer and cited facts highlighted, for the host's journal.
- [ ] Bounded work: evaluation cost is reported and capped, and it never disturbs live frame
      pacing.
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** evidence for a setup equals the result of stepping the replay clock bar by bar and
recording live events. A deliberately look-ahead-contaminated specification is impossible to
express, and the ledger reports every trial.

### P6 — Agent benchmark and program closure

**Scope:** PX1–PX5. **Depends on:** P5. **Status:** proposed.

- [ ] Agent comprehension benchmark (PX2) and rule-translation fidelity benchmark (PX3) added to
      the evidence benchmark subsystem with versioned datasets and question sets.
- [ ] [Public_api.md](../docs/Public_api.md) classifies the perception, setup and evidence surfaces.
- [ ] Milestone evidence: perception-layer screenshots on every executor, the PX1 agreement
      report, PX2 and PX3 results, and recorded performance budgets.

**Exit:** all evaluation targets in the next section are met, or the plan records the measured
shortfall and the decision taken.

## Evaluation program

The program succeeds only when it measurably improves an agent's understanding. Running is not
enough.

- **PX1 — Perception agreement.** Experienced traders annotate a fixed, versioned set of charts
  (swings, zones, trendlines, patterns). Report precision and recall per fact kind and scale.
  Disagreements become documented parameter decisions, never silent tuning against the test set.
- **PX2 — Agent comprehension.** Ask the same factual questions about the same charts in three
  conditions: raw OHLCV, chart image, and Chart Brief with drill-down. Example questions: "nearest
  resistance above price", "major trend direction", "is this a valid flag and why not". Measure
  accuracy, tokens and latency. The hypothesis to prove is that the brief beats both alternatives
  on accuracy at a fraction of the raw-data tokens. Run anonymized and named variants to measure
  memorization effects.
- **PX3 — Rule translation fidelity.** A fixed suite of plain-language strategies. Measure how
  often an agent's specification matches the reference specification, and how often the engine
  correctly rejects ambiguous phrasing.
- **PX4 — Point-in-time correctness.** For every fact kind and setup, outputs at replay clock *t*
  equal outputs computed on data truncated at *t*. This is a required fixture, not a sample.
- **PX5 — Performance.** Release `perf_gate` budgets for tip-update perception cost, brief
  generation, drill-down queries, live setup evaluation, and historical evaluation throughput,
  all with flat retained memory.

## Scope, ownership and architecture rules

| Aeris Charts owns | Hosts own (Aeris Terminal for the platform) |
| --- | --- |
| Pure detection math (`aeris_charts_indicators`) | Agent tool server (for example MCP), model selection and prompts |
| Fact stores, as-of queries, brief, relations, triggers (`aeris_charts_engine`) | Which agent sees which chart, anonymization policy, rate limits |
| Setup specification, validation, live evaluation, events, single-chart evidence | Executing or rejecting trading intents, risk limits, broker connectivity |
| Perception layer projection on every executor | Journals, storage of decision snapshots, cross-symbol scans and portfolio statistics |
| Typed schemas, persistence of settings and specifications | User consent, account policy, and how agent output is presented |
| — | Paper trading, live-versus-evidence drift monitoring, and stopping a strategy when it drifts |

Architecture rules for every batch:

- **No model inside the engine.** Perception is deterministic computation. Nothing calls a model,
  samples randomness or depends on wall-clock time.
- **One owner per fact.** Studies that render structure (swings, zones, fair value gaps) are views of
  perception facts, not parallel implementations.
- **Nothing invented.** Facts state their source and approximations (for example candle-based order
  flow). Unknown aggressor sides and missing depth stay unknown.
- **Bounded everything.** Fact stores, relation graphs, briefs, setups, occurrences and ledgers
  have explicit caps, eviction rules and memory telemetry.
- **Point-in-time by construction.** Detection code receives only data up to the bar it publishes
  for. Equivalence fixtures guard every kind.
- **Shared geometry.** Facts render through the existing drawing contract and ordered frame. No
  executor-specific perception code.
- **Hosts receive behavior.** "Describe this chart", "evaluate this setup" and "show the evidence"
  are single engine operations, not sequences each host must repeat.
- **No new crates** unless a real dependency boundary requires one. Detection math belongs in
  `aeris_charts_indicators`, and perception, setups and evidence in `aeris_charts_engine` modules.

## Risks and non-goals

- **Edge is not guaranteed.** Perception and discipline do not make an unprofitable rule
  profitable. Evidence exists to say "no" as clearly as "yes".
- **Definition risk.** Classical patterns have no single agreed definition. Clause-level output
  and published tolerances make every definition inspectable and debatable, rather than hidden
  behind a confidence number.
- **Overfitting risk moves to the agent.** An agent can iterate specifications quickly. The trial
  ledger and holdout lock make that selection visible. They cannot prevent a host from ignoring
  them.
- **Non-goals:** a scripting language or interpreter, price prediction, an in-engine model, broker
  execution, cross-symbol research infrastructure, and investment advice.

## Sources

- MME-Finance: <https://arxiv.org/abs/2411.03314>
- Do VLMs Truly "Read" Candlesticks? <https://arxiv.org/abs/2604.12659>
- Vision LLM chart audit (April 2026): <https://gist.github.com/roman-rr/c1cd675f7c35b68ae5ac281c30080166>
- Lo, Mamaysky and Wang, Foundations of Technical Analysis: <https://www.nber.org/papers/w7613>
- Jiang, Kelly and Xiu, (Re-)Imag(in)ing Price Trends: <https://economics.yale.edu/sites/default/files/2023-11/The%20Journal%20of%20Finance%20-%202023%20-%20JIANG%20-%20Re%25E2%2580%2590%20Imag%20in%20ing%20Price%20Trends_0.pdf>
- Perceptually Important Points: <https://research.polyu.edu.hk/en/publications/improvement-algorithms-of-perceptually-important-point-identifica/>
- Bulkowski failure rates: <https://thepatternsite.com/FailureRates.html>
- LLM time-series representation survey: <https://arxiv.org/pdf/2402.01801>
- Profit Mirage (information leakage in LLM agents): <https://arxiv.org/html/2510.07920v1>
- Anonymization-first LLM trading: <https://arxiv.org/pdf/2603.17692>
- Deflated Sharpe Ratio: <https://papers.ssrn.com/abstract=2460551>
- TradingView MCP public beta: <https://cryptobriefing.com/tradingview-mcp-server-ai-agents-beta/>
- QuantConnect Mia: <https://www.quantconnect.com/docs/v2/ai-assistance/predefined-agents/mia>
- BacktestBench: <https://arxiv.org/pdf/2605.17937>
