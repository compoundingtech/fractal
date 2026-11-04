// Objectives and budgets as the phone (apps/ios/slo.ts) and the terminal UI
// (crates/fractal-tui/src/ui/objectives.rs) word them, over the decoded native rows. st serves
// each row's tone and forecast; nothing here decides what is red.
import type * as Native from '@fractal/client/schema'
import * as Option from 'effect/Option'

export type SloTone = 'red' | 'green' | 'yellow' | 'quiet'

/** The served tone; a word this client does not know, or none from an older daemon, is quiet. */
export const toneOf = (row: Pick<Native.SloRow, 'tone'>): SloTone =>
  row.tone === 'red' || row.tone === 'green' || row.tone === 'yellow' ? row.tone : 'quiet'

/** What is left of a row's allowance or error budget, or that nothing was measured. */
export const leftText = (row: Pick<Native.SloRow, 'remaining_percent'>): string =>
  Option.match(row.remaining_percent, { onNone: () => 'no data', onSome: left => `${Math.round(left)}% left` })

/** A span as the terminal UI words it: seconds, minutes, hours, then days. */
export const spanText = (ms: number): string => {
  const seconds = Math.floor(Math.max(0, ms) / 1000)
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`
  return `${Math.floor(seconds / 86_400)}d`
}

export interface Forecast { readonly short: string; readonly long: string; readonly runsOut: boolean }

/** A budget's served forecast: short for its list line, long for its detail, and whether it runs out before it resets. */
export const forecastOf = (row: Pick<Native.SloRow, 'forecast_used_percent' | 'runs_out_at_unix_ms'>, now: number): Forecast | null => {
  if (row.runs_out_at_unix_ms !== undefined) {
    const inText = spanText(row.runs_out_at_unix_ms - now)
    return { short: `out in ${inText}`, long: `runs out in ${inText}, before it resets`, runsOut: true }
  }
  if (row.forecast_used_percent !== undefined) {
    const used = Math.round(row.forecast_used_percent)
    return { short: `${used}% by reset`, long: `${used}% used by the reset`, runsOut: false }
  }
  return null
}

/** A list row's second line: its current value, then its forecast, or else its target and window. */
export const secondLine = (row: Native.SloRow, now: number): { readonly text: string; readonly forecast: Forecast | null } => {
  const forecast = forecastOf(row, now)
  return forecast ? { text: `${row.current} · `, forecast } : { text: `${row.current} · target ${row.target} · ${row.window}`, forecast: null }
}

/** Budgets before objectives, each in the order st lists them. */
export const sloGroups = (rows: readonly Native.SloRow[]) =>
  (['budget', 'objective'] as const)
    .map(kind => ({ kind, title: `${kind}s`, rows: rows.filter(row => row.kind === kind) }))
    .filter(group => group.rows.length > 0)

const LEVELS = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']

/** One block character per step: what was left then. A step with no reading is a dot, never a gap that reads as zero. */
export const bars = (history: Pick<Native.SloHistory, 'steps'>, width: number): string =>
  history.steps.slice(-Math.max(1, width))
    .map(step => step === null ? '·' : LEVELS[Math.max(1, Math.round(Math.min(100, Math.max(0, step)) / 100 * 8))])
    .join('')

/** The chart's footer as the terminal UI words it, from the served steps only. */
export const chartSummary = (history: Native.SloHistory): string => {
  const read = history.steps.filter((step): step is number => step !== null)
  if (read.length === 0) return `no reading in these columns · ${history.samples} readings in the period`
  return `low ${Math.round(Math.min(...read))}% · latest ${Math.round(read.at(-1)!)}% · ${history.samples} readings · a column is its lowest reading`
}

/** Only an http or https address is offered as a link. */
export const safeLink = (url: string | undefined): string | null => {
  if (url === undefined || !/^https?:\/\//i.test(url)) return null
  try { return new URL(url).href } catch { return null }
}

/** Dollars from micro-dollars, as the phone's usage screen words them. */
export const money = (microusd: number): string => {
  const dollars = microusd / 1_000_000
  if (microusd === 0) return '$0'
  if (dollars < 0.01) return '<$0.01'
  return dollars < 100 ? `$${dollars.toFixed(2)}` : `$${Math.round(dollars)}`
}

/** An api-key account's month so far; a + when some tokens had no price. No tone: st serves none for credits. */
export const creditsText = (credits: Native.UsageCredits): string =>
  `credits ${money(credits.used_microusd)}${credits.unpriced_tokens > 0 ? '+' : ''} of ${money(credits.budget_microusd)} this month`
