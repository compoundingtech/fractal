import { DateTime, Duration, Schema } from 'effect'
import { describe, expect, expectTypeOf, it } from 'vitest'
import { emit, Instant, mission, missionWire, MissionSchema } from './mod.ts'

const base = {
  id: 'example/dates', state: 'ready', reportTo: { id: 'example/owner' },
  goal: ['Dates are soft metadata; evidence and execution rules remain unchanged.'],
  steps: [{ id: 'work', agentless: true }],
} as const

const invalidInstants = [
  '2026-11-04', '2026-02-30T00:00:00Z', '2027-02-29T00:00:00Z',
  '2026-11-04T24:00:00Z', '2026-11-04T00:00:00', '2026-11-04T00:00:00-00:00',
  '2026-11-04T00:00:60Z', '2026-11-04T00:00:00.0000Z', '2026-11-04T00:00:00.1234Z',
  '2026-11-04T00:00:00+24:00', '2026-11-04T00:00:00+01:60',
  '2026-11-04t00:00:00z', '2026-11-04 00:00:00Z', 'next Thursday',
]

describe('mission date declarations', () => {
  it('decodes an explicit-offset instant to rich UTC and encodes canonical milliseconds', () => {
    const at = Schema.decodeSync(Instant)('2028-02-29T01:00:00.123+01:00')
    expectTypeOf(at).toEqualTypeOf<DateTime.Utc>()
    expect(DateTime.isDateTime(at)).toBe(true)
    expect(DateTime.formatIso(at)).toBe('2028-02-29T00:00:00.123Z')
    expect(Schema.encodeSync(Instant)(at)).toBe('2028-02-29T00:00:00.123Z')
    expect(Schema.decodeSync(Instant)(at)).toEqual(at)
  })

  it('supports elapsed Duration arithmetic without putting a clock on the native wire', () => {
    const targetAt = DateTime.makeUnsafe('2026-03-28T11:00:00Z')
    const at = DateTime.addDuration(targetAt, Duration.hours(24))
    const input = { ...base, targetAt, milestones: [{ id: 'review', at }] }
    const decoded = Schema.decodeSync(MissionSchema)(input)
    expect(DateTime.formatIso(decoded.targetAt!)).toBe('2026-03-28T11:00:00.000Z')
    expect(missionWire(input).targetAt).toBe('2026-03-28T11:00:00.000Z')
    expect(missionWire(input).milestones).toEqual([{ id: 'review', at: '2026-03-29T11:00:00.000Z' }])
    const kdl = emit([mission(input)])
    expect(kdl).toContain('target-at="2026-03-28T11:00:00.000Z"')
    expect(kdl).toContain('milestone "review" at="2026-03-29T11:00:00.000Z"')
    expect(missionWire(input).timeout).toBeUndefined()
  })

  it.each(invalidInstants)('rejects out-of-profile instant %s at both declaration fields', (at) => {
    expect(() => Schema.decodeUnknownSync(Instant)(at)).toThrow()
    expect(() => Schema.decodeUnknownSync(MissionSchema)({ ...base, targetAt: at })).toThrow()
    expect(() => Schema.decodeUnknownSync(MissionSchema)({ ...base, milestones: [{ id: 'review', at }] })).toThrow()
  })

  it('requires unique local milestone IDs and chronological order of instants, not offset text', () => {
    const first = { id: 'first', at: '2026-11-04T23:30:00-02:00' as const }
    const second = { id: 'second', at: '2026-11-05T02:00:00Z' as const }
    expect(missionWire({ ...base, milestones: [first, second] }).milestones).toEqual([
      { id: 'first', at: '2026-11-05T01:30:00.000Z' },
      { id: 'second', at: '2026-11-05T02:00:00.000Z' },
    ])
    for (const milestones of [
      [second, first], [first, first], [first, { ...second, id: 'first' }],
      [first, { ...second, at: '2026-11-05T01:30:00Z' }],
      [{ at: first.at }], [{ id: '', at: first.at }], [{ id: 'bad id', at: first.at }],
    ]) {
      expect(() => Schema.decodeUnknownSync(MissionSchema)({ ...base, milestones })).toThrow()
    }
    expect(missionWire({ ...base, milestones: [
      { id: 'zero', at: '2026-11-05T00:00:00Z' },
      { id: 'one', at: '2026-11-05T00:00:00.001Z' },
    ] }).milestones).toHaveLength(2)
  })

  it('leaves absent date fields absent and adds no empty KDL metadata', () => {
    const wire = missionWire(base)
    expect(wire).not.toHaveProperty('targetAt')
    expect(wire).not.toHaveProperty('milestones')
    expect(emit([mission({ ...base, milestones: [] })])).toBe(emit([mission(base)]))
  })
})
