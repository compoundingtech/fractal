// Objectives and budgets in the Usage tab, as the Fractal terminal UI's Usage tab shows them
// (crates/fractal-tui/src/ui/objectives.rs). The rows come from st's `slo.list` read, which also
// says how urgent each is (`tone`) and, for a budget with enough recent readings, what it will
// have used when it resets (`forecast_used_percent`) and when it runs out first
// (`runs_out_at_unix_ms`). Nothing here decides what is red: st serves that.
import type { SloHistory, SloRow } from '../../clients/typescript/fractal-client';

export type SloTone = 'red' | 'green' | 'yellow' | 'quiet';

/** What is left of a row's allowance or error budget, or that nothing was measured. */
export const leftText = (row: Pick<SloRow, 'remaining_percent'>) => row.remaining_percent == null ? 'no data' : `${Math.round(row.remaining_percent)}% left`;

/** A span as the terminal UI words it: seconds, minutes, hours, then days. */
export function spanText(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}

export type Forecast = { short: string; long: string; runsOut: boolean };

/** A budget's forecast: short for a list line, long for its detail, and whether it runs out before it resets. */
export function forecastOf(row: Pick<SloRow, 'forecast_used_percent' | 'runs_out_at_unix_ms'>, now: number): Forecast | null {
  if (row.runs_out_at_unix_ms != null) {
    const inText = spanText(row.runs_out_at_unix_ms - now);
    return { short: `out in ${inText}`, long: `runs out in ${inText}, before it resets`, runsOut: true };
  }
  if (row.forecast_used_percent != null) {
    const used = Math.round(row.forecast_used_percent);
    return { short: `${used}% by reset`, long: `${used}% used by the reset`, runsOut: false };
  }
  return null;
}

/** The second line of a list row: its current value, then its forecast, or else its target and window. */
export const secondLine = (row: SloRow, now: number) => {
  const forecast = forecastOf(row, now);
  return forecast ? { text: `${row.current} · `, forecast } : { text: `${row.current} · target ${row.target} · ${row.window}`, forecast: null };
};

/** Budgets before objectives, each in the order st lists them. */
export function sloGroups(rows: readonly SloRow[]): Array<{ kind: 'budget' | 'objective'; title: string; rows: SloRow[] }> {
  return (['budget', 'objective'] as const)
    .map(kind => ({ kind, title: `${kind}s`, rows: rows.filter(row => row.kind === kind) }))
    .filter(group => group.rows.length > 0);
}

const LEVELS = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/** A row of block characters, one per step: what was left then. A step with no reading is a dot, never a gap that reads as zero. */
export function bars(history: Pick<SloHistory, 'steps'>, width: number): string {
  const steps = history.steps.slice(-Math.max(1, width));
  return steps.map(step => step == null ? '·' : LEVELS[Math.max(1, Math.round(Math.min(100, Math.max(0, step)) / 100 * 8))]).join('');
}

/** Only an http or https address is offered as a link. */
export const safeLink = (url: string | undefined | null): string | null => url && /^https?:\/\//i.test(url) ? url : null;
