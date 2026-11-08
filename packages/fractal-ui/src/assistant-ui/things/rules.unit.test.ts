import { describe, expect, it } from 'vitest'
import { actPaint, bareAge, cardAge, compareThings, rowAge, sortThings, thingLabel, thingRank } from './rules.ts'
import type { Observed, ThingFace } from './face.ts'

const now = Date.UTC(2026, 10, 9, 12, 0, 0)
const minute = 60_000
const pr: ThingFace = { kind: 'Pull request', icon: 'pull-request', title: 'Fix login', id: '#214', mark: { shape: 'hollow', word: 'open' }, at: { at: now - 4 * minute, verb: 'updated' } }
const known = (value: ThingFace, extra: { readonly freshness?: 'live' | 'stale'; readonly readOnly?: boolean; readonly observedAt?: number } = {}): Extract<Observed<ThingFace>, { _tag: 'Known' }> =>
  ({ _tag: 'Known', value, observedAt: extra.observedAt ?? now, freshness: extra.freshness ?? 'live', ...(extra.readOnly === undefined ? {} : { readOnly: extra.readOnly }) })

describe('thing rules', () => {
  it('shows bare formatRelative ages on rows; the card keeps the verb', () => {
    expect(bareAge(now - 4 * minute, now)).toBe('4m')
    expect(bareAge(now - 10_000, now)).toBe('just now')
    expect(rowAge(known(pr), now)).toBe('4m')
    expect(cardAge(pr, now)).toBe('updated 4m')
    const { at: _at, ...timeless } = pr
    expect(rowAge(known(timeless), now)).toBeUndefined()
  })

  it('reads a stale row as "seen …" from its observation, whatever its own moment', () => {
    expect(rowAge(known(pr, { freshness: 'stale', observedAt: now - 25 * minute }), now)).toBe('seen 25m')
  })

  it('keeps amber for asks you can act on: read-only asks lose colour, failures and stale rows keep it', () => {
    const ask: ThingFace = { ...pr, act: { tone: 'act', label: 'changes requested' } }
    const failed: ThingFace = { ...pr, act: { tone: 'failed', label: 'checks failing' } }
    expect(actPaint(known(pr))).toBeUndefined()
    expect(actPaint(known(ask))).toBe('act')
    expect(actPaint(known(ask, { readOnly: true }))).toBe('quiet')
    expect(actPaint(known(failed, { readOnly: true }))).toBe('failed')
    expect(actPaint(known(ask, { freshness: 'stale' }))).toBe('act')
  })

  it('names a row with everything a narrow row may drop', () => {
    expect(thingLabel(known(pr), now)).toBe('Pull request #214 Fix login, open, 4m')
    expect(thingLabel(known({ ...pr, act: { tone: 'failed', label: 'checks failing' } }), now)).toBe('Pull request #214 Fix login, checks failing, 4m')
  })

  it('orders sections failed and needs-you first, then active, then newest', () => {
    const item = (key: string, face: ThingFace, readOnly = false) => ({ key, observed: known(face, { readOnly }) as Observed<ThingFace> })
    const items = [
      item('old', { ...pr, at: { at: now - 60 * minute, verb: 'updated' } }),
      item('new', { ...pr, at: { at: now - minute, verb: 'updated' } }),
      item('active', { ...pr, mark: { shape: 'filled', word: 'running' }, at: { at: now - 90 * minute, verb: 'started' } }),
      item('failed', { ...pr, act: { tone: 'failed', label: 'exit 101' }, at: { at: now - 120 * minute, verb: 'finished' } }),
      item('quiet', { ...pr, act: { tone: 'act', label: 'review requested' }, at: { at: now - 2 * minute, verb: 'updated' } }, true),
      { key: 'loading', observed: { _tag: 'Loading', hint: { kind: 'Pull request', icon: 'pull-request' } } as Observed<ThingFace> },
    ]
    expect(sortThings(items).map(each => each.key)).toEqual(['failed', 'active', 'new', 'quiet', 'old', 'loading'])
    expect(thingRank(items[4]!.observed)).toBe(2)
    expect(compareThings(items[3]!.observed, items[1]!.observed)).toBeLessThan(0)
  })
})
