import type { UsageMeterRow, UsageMeterTone } from '../UsageMeterTable'

type State = 'current' | 'stale' | 'failed'
const readings: readonly [string, string, string, number, number, string, UsageMeterTone, State][] = [
  ['north', 'North studio', 'gh/north-studio', 72, 41, '1h 12m', 'warning', 'current'],
  ['harbor', 'Harbor lab', 'cx/harbor-lab', 100, 86, '2d 4h', 'exhausted', 'current'],
  ['meadow', 'Meadow research', 'cl/meadow-research', 34, 19, 'Unreported', 'ok', 'current'],
  ['ventures', 'Fieldworks ventures', 'gh/fieldworks-ventures', 95, 63, '5d', 'critical', 'current'],
  ['quarry', 'Quarry studio', 'cx/quarry-studio', 81, 47, '3h', 'critical', 'stale'],
  ['cedar', 'Cedar workshop', 'cl/cedar-workshop', 88, 51, '8h', 'critical', 'current'],
  ['ridge', 'Ridge design', 'cx/ridge-design', 60, 28, '2h 30m', 'warning', 'current'],
  ['slate', 'Slate partners', 'gh/slate-partners', 0, 0, 'Unreported', 'unknown', 'failed'],
  ['brook', 'Brook analytics', 'gh/brook-analytics', 42, 17, '3d', 'ok', 'current'],
  ['elm', 'Elm services', 'cl/elm-services', 26, 11, '45m', 'ok', 'current'],
  ['dune', 'Dune tooling', 'cx/dune-tooling', 18, 9, '6h', 'ok', 'current'],
  ['pine', 'Pine systems', 'gh/pine-systems', 9, 4, '12h', 'ok', 'current'],
  ['lake', 'Lake sandbox', 'cl/lake-sandbox', 0, 2, '1d', 'ok', 'current'],
]

export const quotaPriorityAccounts: readonly UsageMeterRow[] = readings.map(([id, label, identity, primary, secondary, reset, tone, state]) => {
  const failed = state === 'failed'
  const governing = failed ? { label: '5h', text: 'fail', tone } : { label: id === 'harbor' ? '1w' : '5h', text: `${primary}%`, fraction: primary / 100, tone }
  const comparison = failed ? { label: '1w', text: 'fail', tone } : { label: id === 'harbor' ? '5h' : '1w', text: `${secondary}%`, fraction: secondary / 100, tone: secondary >= 80 ? 'critical' as const : 'ok' as const }
  const note = tone === 'exhausted' ? 'Exhausted · requests resume after reset' : failed ? 'Read failed · last poll returned an error' : undefined
  return {
    id, label, identity, governing, comparison,
    reset, omitted: 1, stale: state === 'stale',
    ...(note === undefined ? {} : { note }),
    detail: {
      facts: [{ label: 'Account', value: identity }, { label: 'Plan', value: id === 'harbor' ? 'Personal' : 'Team' }, { label: 'State', value: tone === 'exhausted' ? 'Exhausted' : state === 'stale' ? 'Stale · last-known values' : failed ? 'Failed' : 'Current' }],
      windows: [
        { id: `${id}-primary`, label: 'Primary', meter: governing, facts: [] },
        { id: `${id}-secondary`, label: 'Secondary', meter: comparison, facts: [] },
        { id: `${id}-month`, label: 'Monthly', meter: failed ? { label: '1mo', text: 'fail', tone } : { label: '1mo', text: id === 'north' ? '18%' : '8%', fraction: id === 'north' ? .18 : .08, tone: 'ok' }, facts: [] },
      ],
      resets: [{ label: 'Primary reset', value: reset }],
    },
  }
})
