import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { MissionRunSummary, decodeUnknownSync } from '@fractal/client/schema'
import * as Option from 'effect/Option'
import { MissionEtaLabel, formatMissionEta, type MissionEta } from '../../../../packages/fractal-ui/src/assistant-ui/MissionEta.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const eta: MissionEta = { remaining_low_ms: 720_000, remaining_high_ms: 1_080_000,
  basis: { method: 'critical-path', samples: 12 }, as_of: 1_794_000_000_000 }

describe('served mission ETA', () => {
  it('formats the served range and a single value without a countdown', () => {
    expect(formatMissionEta(eta)).toBe('~12–18 min left')
    expect(formatMissionEta({ ...eta, remaining_high_ms: 720_000 })).toBe('~12 min left')
    expect(formatMissionEta({ ...eta, remaining_low_ms: 10_000, remaining_high_ms: 30_000 })).toBe('~10–30 sec left')
    expect(formatMissionEta({ ...eta, remaining_low_ms: 7_200_000, remaining_high_ms: 10_800_000 })).toBe('~2–3 hr left')
    expect(formatMissionEta({ ...eta, remaining_low_ms: 0, remaining_high_ms: 0 })).toBe('~0 sec left')
  })

  it('ages the observation description without reducing the served range', () => {
    const fresh = renderToStaticMarkup(createElement(MissionEtaLabel, { eta, now: eta.as_of }))
    const aged = renderToStaticMarkup(createElement(MissionEtaLabel, { eta, now: eta.as_of + 300_000 }))
    expect(fresh).toContain('~12–18 min left')
    expect(aged).toContain('~12–18 min left')
    expect(aged).toContain('Critical-path')
    expect(aged).toContain('12 retained samples')
    expect(aged).toContain('served as of')
    expect(aged).not.toBe(fresh)
  })
  it.each([null, undefined])('renders nothing for missing ETA %s', value => {
    expect(formatMissionEta(value)).toBeNull()
    expect(renderToStaticMarkup(createElement(MissionEtaLabel, { eta: value, now: eta.as_of }))).toBe('')
  })

  it.each([{}, { eta: null }])('decodes older and unsampled runs without an ETA', extra => {
    const run = decodeUnknownSync(MissionRunSummary)({ id: 'mission-run/example/run', requester: 'agent/example-worker',
      status: 'running', phase: 'working', state_since: '2026-11-04T12:00:00Z',
      progress: { done: 2, total: 5 }, current_steps: [], must_act: 'agent', ...extra })
    expect(Option.isNone(run.eta)).toBe(true)
  })
})
