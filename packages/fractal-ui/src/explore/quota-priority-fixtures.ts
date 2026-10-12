import type { UsageMeterRow, UsageMeterTone } from '../UsageMeterTable'

const readings: readonly [string, string, string, number, number, string, UsageMeterTone][] = [
  ['north', 'North studio', 'gh/north-studio', 72, 41, '1h 12m', 'warning'],
  ['harbor', 'Harbor lab', 'cx/harbor-lab', 100, 86, '2d 4h', 'exhausted'],
  ['meadow', 'Meadow research', 'cl/meadow-research', 34, 19, 'Unreported', 'ok'],
  ['ventures', 'Fieldworks ventures', 'gh/fieldworks-ventures', 95, 63, '5d', 'critical'],
  ['cedar', 'Cedar workshop', 'cl/cedar-workshop', 88, 51, '8h', 'critical'],
  ['ridge', 'Ridge design', 'cx/ridge-design', 60, 28, '2h 30m', 'warning'],
  ['brook', 'Brook analytics', 'gh/brook-analytics', 42, 17, '3d', 'ok'],
  ['elm', 'Elm services', 'cl/elm-services', 26, 11, '45m', 'ok'],
  ['dune', 'Dune tooling', 'cx/dune-tooling', 18, 9, '6h', 'ok'],
  ['pine', 'Pine systems', 'gh/pine-systems', 9, 4, '12h', 'ok'],
  ['lake', 'Lake sandbox', 'cl/lake-sandbox', 0, 2, '1d', 'ok'],
]

export const quotaPriorityAccounts: readonly UsageMeterRow[] = readings.map(([id, label, identity, primary, secondary, reset, tone]) => {
  const governing = { label: id === 'harbor' ? '1w' : '5h', text: `${primary}%`, fraction: primary / 100, tone }
  const comparison = { label: id === 'harbor' ? '5h' : '1w', text: `${secondary}%`, fraction: secondary / 100, tone: secondary >= 80 ? 'critical' as const : 'ok' as const }
  return {
    id, label, identity, governing, comparison,
    reset, omitted: 1, stale: false,
    ...(tone === 'exhausted' ? { note: 'Exhausted · requests resume after reset' } : {}),
    detail: {
      facts: [{ label: 'Account', value: identity }, { label: 'Plan', value: id === 'harbor' ? 'Personal' : 'Team' }, { label: 'State', value: tone === 'exhausted' ? 'Exhausted' : 'Current' }],
      windows: [
        { id: `${id}-primary`, label: 'Primary', meter: governing, facts: [] },
        { id: `${id}-secondary`, label: 'Secondary', meter: comparison, facts: [] },
        { id: `${id}-month`, label: 'Monthly', meter: { label: '1mo', text: id === 'north' ? '18%' : '8%', fraction: id === 'north' ? .18 : .08, tone: 'ok' }, facts: [] },
      ],
      resets: [{ label: 'Primary reset', value: reset }],
    },
  }
})
