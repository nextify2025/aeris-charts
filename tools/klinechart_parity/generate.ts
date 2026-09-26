/**
 * Generates the KLineChart indicator parity fixture used by
 * `crates/aeris_charts_indicators/tests/klinechart_parity.rs`.
 *
 * The expected values come from KLineChart's own `calc` functions, executed unmodified, so the Rust
 * ports in `aeris_charts_indicators::klinechart` are checked against the reference implementation
 * rather than against a re-derivation of its formulas.
 *
 * Usage (from the repository root):
 *
 *   git clone https://github.com/klinecharts/KLineChart.git /tmp/KLineChart
 *   git -C /tmp/KLineChart checkout 044773a57fbbb8fa70f8bb00661a87f9089b5d29   # v10.0.3
 *   npx tsx tools/klinechart_parity/generate.ts /tmp/KLineChart \
 *     > crates/aeris_charts_indicators/tests/fixtures/klinechart_parity.json
 *
 * KLineChart is Copyright (c) 2019 lihu and licensed under the Apache License, Version 2.0.
 */

import { execFileSync } from 'node:child_process'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

interface Bar {
  timestamp: number
  open: number
  high: number
  low: number
  close: number
  volume: number
  turnover: number
}

// Each entry: KLineChart indicator source file, then extra parameter sets beyond the template's
// own `calcParams` defaults. `null` means the indicator has no parameters.
const CASES: Array<[string, number[][] | null]> = [
  ['movingAverage', [[3, 7]]],
  ['exponentialMovingAverage', [[5, 10]]],
  ['simpleMovingAverage', [[9, 3]]],
  ['bullAndBearIndex', [[2, 4, 8, 16]]],
  ['volume', [[3, 7]]],
  ['movingAverageConvergenceDivergence', [[6, 13, 5]]],
  ['bollingerBands', [[26, 2.5]]],
  ['stoch', [[14, 3, 5]]],
  ['relativeStrengthIndex', [[14]]],
  ['bias', [[5, 10]]],
  ['brar', [[13]]],
  ['commodityChannelIndex', [[14]]],
  ['currentRatio', [[13, 5, 10, 20, 30]]],
  ['differentOfMovingAverage', [[5, 20, 6]]],
  ['directionalMovementIndex', [[10, 4]]],
  ['easeOfMovementValue', [[7, 5]]],
  ['momentum', [[6, 3]]],
  ['onBalanceVolume', [[10]]],
  ['priceAndVolumeTrend', null],
  ['psychologicalLine', [[6, 3]]],
  ['rateOfChange', [[6, 3]]],
  ['stopAndReverse', [[4, 2, 30]]],
  ['tripleExponentiallySmoothedAverage', [[6, 4]]],
  ['volumeRatio', [[13, 3]]],
  ['williamsR', [[5, 21]]],
  ['awesomeOscillator', [[3, 10]]],
  ['averagePrice', null]
]

function mulberry32 (seed: number): () => number {
  let a = seed
  return () => {
    a |= 0
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function gaussian (rand: () => number): number {
  const u = Math.max(rand(), 1e-12)
  const v = rand()
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v)
}

const round2 = (v: number): number => Math.round(v * 100) / 100

/**
 * A random walk with deliberate edge cases: a strictly rising run (RSI loss average reaches 0), a
 * strictly falling run, a flat run with zero volume (zero ranges and zero divisors), and repeated
 * closes. Prices are rounded to cents so exact equalities occur the way they do in real quotes.
 */
function randomWalk (count: number, seed: number): Bar[] {
  const rand = mulberry32(seed)
  const bars: Bar[] = []
  let prevClose = 100
  for (let i = 0; i < count; i++) {
    let open: number
    let close: number
    let high: number
    let low: number
    let volume = Math.round((2e5 + rand() * 4.8e6) / 100) * 100
    if (i >= 100 && i < 120) {
      open = prevClose
      close = round2(prevClose + 0.3 + rand())
      high = round2(close + rand() * 0.2)
      low = round2(open - rand() * 0.2)
    } else if (i >= 200 && i < 215) {
      open = prevClose
      close = round2(prevClose - 0.3 - rand())
      high = round2(open + rand() * 0.2)
      low = round2(close - rand() * 0.2)
    } else if (i >= 250 && i < 256) {
      open = prevClose
      close = prevClose
      high = prevClose
      low = prevClose
      if (i === 251 || i === 253) volume = 0
    } else {
      open = round2(prevClose * (1 + gaussian(rand) * 0.005))
      close = i % 37 === 0 ? prevClose : round2(open * (1 + gaussian(rand) * 0.02))
      high = round2(Math.max(open, close) * (1 + Math.abs(gaussian(rand)) * 0.01))
      low = round2(Math.min(open, close) * (1 - Math.abs(gaussian(rand)) * 0.01))
    }
    const turnover = round2(volume * ((high + low + close) / 3))
    bars.push({ timestamp: 1_700_000_000_000 + i * 86_400_000, open, high, low, close, volume, turnover })
    prevClose = close
  }
  return bars
}

function constant (count: number): Bar[] {
  return Array.from({ length: count }, (_, i) => ({
    timestamp: 1_700_000_000_000 + i * 86_400_000,
    open: 10,
    high: 10,
    low: 10,
    close: 10,
    volume: 1000,
    turnover: 10000
  }))
}

const DATASETS: Record<string, Bar[]> = {
  random_walk: randomWalk(360, 20260926),
  short: randomWalk(12, 7),
  constant: constant(40)
}

async function main (): Promise<void> {
  const root = process.argv[2]
  if (root === undefined) {
    throw new Error('usage: tsx tools/klinechart_parity/generate.ts <path-to-KLineChart-checkout>')
  }
  const commit = execFileSync('git', ['-C', root, 'rev-parse', 'HEAD']).toString().trim()
  const pkg = await import(pathToFileURL(resolve(root, 'package.json')).href, { with: { type: 'json' } })

  const cases: unknown[] = []
  for (const [file, extraParams] of CASES) {
    const url = pathToFileURL(resolve(root, 'src/extension/indicator', `${file}.ts`)).href
    const template = (await import(url)).default
    const paramSets: number[][] = extraParams === null ? [[]] : [template.calcParams, ...extraParams]
    for (const params of paramSets) {
      const figures = typeof template.regenerateFigures === 'function' ? template.regenerateFigures(params) : template.figures
      const keys: string[] = figures.map((f: { key: string }) => f.key)
      for (const [dataset, bars] of Object.entries(DATASETS)) {
        const rows = await template.calc(bars, { calcParams: params, figures, extendData: template.extendData })
        const outputs: Record<string, Array<number | null>> = {}
        for (const key of keys) {
          outputs[key] = rows.map((row: Record<string, unknown>) => {
            const value = row?.[key]
            return typeof value === 'number' ? value : null
          })
        }
        cases.push({ indicator: template.name, file, dataset, params, outputs })
      }
    }
  }

  const datasets: Record<string, Record<string, number[]>> = {}
  for (const [name, bars] of Object.entries(DATASETS)) {
    datasets[name] = {
      open: bars.map((b) => b.open),
      high: bars.map((b) => b.high),
      low: bars.map((b) => b.low),
      close: bars.map((b) => b.close),
      volume: bars.map((b) => b.volume),
      turnover: bars.map((b) => b.turnover)
    }
  }

  process.stdout.write(JSON.stringify({ klinechart: { version: pkg.default.version, commit }, datasets, cases }))
  process.stdout.write('\n')
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
