import type { UsageMeterRow, UsageMeterTone } from './usage-meter-types.ts'

type State = 'current' | 'stale' | 'failed'
/** Synthetic accounts, most pressing first (ranking is the caller's policy): id, label, identity, primary %, secondary %, reset, compact reset, tone, state. */
const readings: readonly [string, string, string, number, number, string, string, UsageMeterTone, State][] = [
  ['harbor', 'Harbor lab', 'cx/harbor-lab', 100, 86, 'in 2d 4h', '2d', 'exhausted', 'current'],
  ['ventures', 'Fieldworks ventures', 'gh/fieldworks-ventures', 95, 63, 'in 5d', '5d', 'critical', 'current'],
  ['cedar', 'Cedar workshop', 'cl/cedar-workshop', 88, 51, 'in 8h', '8h', 'critical', 'current'],
  ['quarry', 'Quarry studio', 'cx/quarry-studio', 81, 47, 'in 3h', '3h', 'critical', 'stale'],
  ['north', 'North studio', 'gh/north-studio', 72, 41, 'in 1h 12m', '1h', 'warning', 'current'],
  ['ridge', 'Ridge design', 'cx/ridge-design', 60, 28, 'in 2h 30m', '2h', 'warning', 'current'],
  ['brook', 'Brook analytics', 'gh/brook-analytics', 42, 17, 'in 3d', '3d', 'ok', 'current'],
  ['meadow', 'Meadow research', 'cl/meadow-research', 34, 19, 'Unreported', '—', 'ok', 'current'],
  ['elm', 'Elm services', 'cl/elm-services', 26, 11, 'in 45m', '45m', 'ok', 'current'],
  ['dune', 'Dune tooling', 'cx/dune-tooling', 18, 9, 'in 6h', '6h', 'ok', 'current'],
  ['pine', 'Pine systems', 'gh/pine-systems', 9, 4, 'in 12h', '12h', 'ok', 'current'],
  ['lake', 'Lake sandbox', 'cl/lake-sandbox', 0, 2, 'in 1d', '1d', 'ok', 'current'],
  ['slate', 'Slate partners', 'gh/slate-partners', 0, 0, 'Unreported', '—', 'unknown', 'failed'],
]

export const quotaFooterAccounts: readonly UsageMeterRow[] = readings.map(([id, label, identity, primary, secondary, reset, compactReset, tone, state]) => {
  const failed = state === 'failed'
  const primaryLabel = id === 'harbor' ? '1w' : '5h'
  const secondaryLabel = id === 'harbor' ? '5h' : '1w'
  const governing = failed ? { label: primaryLabel, text: 'Read failed', tone } : { label: primaryLabel, text: `${primary}%`, fraction: primary / 100, tone }
  const comparison = failed ? { label: secondaryLabel, text: 'Read failed', tone } : { label: secondaryLabel, text: `${secondary}%`, fraction: secondary / 100, tone: secondary >= 80 ? 'critical' as const : 'ok' as const }
  const monthly = failed ? { label: '1mo', text: 'Read failed', tone } : { label: '1mo', text: id === 'north' ? '18%' : '8%', fraction: id === 'north' ? 0.18 : 0.08, tone: 'ok' as const }
  const note = tone === 'exhausted' ? 'Exhausted · requests resume after reset' : failed ? 'Read failed · last poll returned an error' : undefined
  return {
    id, label, identity, governing, comparison, reset, compactReset, omitted: 1, stale: state === 'stale',
    ...(note === undefined ? {} : { note }),
    detail: {
      facts: [{ label: 'Account', value: identity }, { label: 'Plan', value: id === 'harbor' ? 'Personal' : 'Team' }, { label: 'State', value: tone === 'exhausted' ? 'Exhausted' : state === 'stale' ? 'Stale · last-known values' : failed ? 'Failed' : 'Current' }],
      windows: [
        { id: `${id}-primary`, label: 'Primary', meter: governing, facts: [] },
        { id: `${id}-secondary`, label: 'Secondary', meter: comparison, facts: [] },
        { id: `${id}-month`, label: 'Monthly', meter: monthly, facts: [] },
      ],
      resets: [{ label: 'Primary reset', value: reset }],
    },
  }
})
