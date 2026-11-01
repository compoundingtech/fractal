import { Attention, decodeUnknownSync, type AttentionEncoded } from '@fractal/client/schema'
import { describe, expect, it } from 'vitest'
import { alertsTabRows, alertTabMarkers } from './alertsTab.ts'

const now = Date.parse('2026-11-01T12:00:00Z')
const item = (fields: Partial<AttentionEncoded> = {}) => decodeUnknownSync(Attention, 'strict')({
  id: 'attention/ask', kind: 'attention', revision: 'r1', attention_kind: 'person-step',
  title: 'Ship it?', detail: '', priority: 'normal', state: 'open', alert: true,
  person_id: 'person/avery', source_id: 'step-run/release/deploy',
  requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z', actions: ['work.done'],
  presentation: { tier: 'stopped', kind: 'request', agent: null, raised_by: null, waiting: null },
  ...fields,
})
const presented = (tier: 'stopped' | 'today' | 'later', kind: 'request' | 'review' | 'fault' = 'request') =>
  ({ tier, kind, agent: null, raised_by: null, waiting: null })

describe('Alerts tab projection', () => {
  it('uses served tiers and preserves served order within each tier', () => {
    const later = item({ id: 'attention/later', presentation: presented('later') })
    const today = item({ id: 'attention/today', presentation: presented('today') })
    const first = item({ id: 'attention/first' })
    const second = item({ id: 'attention/second' })
    expect(alertsTabRows([later, first, today, second], now).map(row => row.id))
      .toEqual([first.id, second.id, today.id, later.id])
  })

  it('keeps open served faults, but excludes resolved cards, updates and messages', () => {
    const fault = item({ attention_kind: 'fault', actions: [], presentation: presented('today', 'fault') })
    const resolved = item({ state: 'resolved' })
    const message = item({ attention_kind: 'unread-message', alert: false })
    const update = item({ alert: false })
    const rows = alertsTabRows([fault, resolved, message, update], now)
    expect(rows.map(row => row.glyph)).toEqual(['!'])
    expect(alertTabMarkers({ _tag: 'Observed', freshness: 'live', value: [fault] }))
      .toEqual({ questionCount: 0, needsYouCount: 0, status: { glyph: '!', label: 'Fault reported' } })
  })

  it('marks harness questions even without a reply port, alongside answerable requests', () => {
    const question = item()
    const review = item({ attention_kind: 'human-gate', actions: ['review.approve'], presentation: presented('stopped', 'review') })
    const terminal = item({ attention_kind: 'harness-prompt', actions: [] })
    const feed = { _tag: 'Observed', freshness: 'stale', value: [question, review, terminal] } as const
    expect(alertsTabRows(feed.value, now).map(row => row.glyph)).toEqual(['?', '◆', '?'])
    expect(alertTabMarkers(feed)).toEqual({ questionCount: 2, needsYouCount: 1 })
    expect(alertTabMarkers({ _tag: 'Waiting' })).toEqual({ questionCount: 0, needsYouCount: 0 })
    expect(alertTabMarkers({ _tag: 'Unavailable', reason: 'unsupported', detail: 'Unsupported' })).toEqual({ questionCount: 0, needsYouCount: 0 })
  })

  it('shows only served raised-by and waiting facts, plus a paired deadline/default answer', () => {
    const served = item({
      requester_id: 'agent/not-the-served-label', mission_id: 'mission/release', conversation_id: 'agent/builder',
      deadline_at: '2026-11-01T14:00:00Z', default_answer: { id: 'skip', label: 'Skip' },
      presentation: { ...presented('stopped'), raised_by: { id: 'agent/builder', label: 'builder' },
        waiting: { path: 'release/deploy', step: 'step-run/release/deploy' } },
    })
    const row = alertsTabRows([served], now)[0]!
    expect(row.line2).toBe('builder · release/deploy · answer within 2h, or “Skip” applies')
    expect(row.conversation).toBe('agent/builder')
    expect(row.mission).toBe('mission/release')
    expect(alertsTabRows([item({ deadline_at: '2026-11-01T14:00:00Z' })], now)[0]!.line2).toBe('')
    expect(alertsTabRows([item({ default_answer: { id: 'skip', label: 'Skip' } })], now)[0]!.line2).toBe('')
    expect(alertsTabRows([item({ requester_id: 'agent/not-served', step_run_id: 'step-run/not-served' })], now)[0]!.line2).toBe('')
  })
})
