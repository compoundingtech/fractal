import type { PersonAvailability } from '@fractal/client'
import { describe, expect, it } from 'vitest'
import { availabilityFacts, formatInstant } from './availabilityModel.ts'

const zone = { locale: 'en-GB', timeZone: 'Europe/Berlin' }
// 10:00 in Berlin on a Thursday.
const now = Date.parse('2026-11-12T09:00:00Z')
const iso = (minutes: number) => new Date(now + minutes * 60_000).toISOString()
const base: PersonAvailability = {
  person: 'person/robin', status: 'available', status_source: 'presence', status_until: null,
  presence: 'present', presence_observed_at: iso(-2), interruptions: 'all', interruptions_until: null,
  deadline_clock: 'run', deadline_clock_since: null, deadline_clock_until: null,
}
const note = { author: 'person/robin', person: 'person/robin', revision: 'r1', text: 'Focus on the release.', time: iso(-30) }

describe('formatInstant', () => {
  it('names today by its clock time, tomorrow by name, and later days by date', () => {
    expect(formatInstant(Date.parse(iso(270)), now, zone)).toBe('14:30')
    expect(formatInstant(Date.parse('2026-11-13T08:00:00Z'), now, zone)).toBe('tomorrow 9:00')
    expect(formatInstant(Date.parse('2026-11-20T08:00:00Z'), now, zone)).toBe('20 Nov 9:00')
  })
})

describe('availabilityFacts', () => {
  it('shows a presence-derived status, the defaults and no note', () => {
    expect(availabilityFacts({ availability: base, note: undefined }, now, zone)).toEqual([
      { label: 'Status', value: 'Available', detail: 'Follows your presence: present 2m ago.' },
      { label: 'Interruptions', value: 'All asks' },
      { label: 'Deadline clock', value: 'Running' },
      { label: 'Note', value: 'No note' },
    ])
  })

  it('shows overrides with their bounds, and the note with its expiry', () => {
    const away: PersonAvailability = {
      ...base, status: 'away', status_source: 'override', status_until: iso(90), presence: 'idle',
      interruptions: 'urgent', interruptions_until: iso(60), deadline_clock: 'hold-routine', deadline_clock_since: iso(-15), deadline_clock_until: '2026-11-13T08:00:00Z',
    }
    expect(availabilityFacts({ availability: away, note: { ...note, expires_at: iso(120) } }, now, zone)).toEqual([
      { label: 'Status', value: 'Away · until 11:30', detail: 'Set by you; presence is idle 2m ago.' },
      { label: 'Interruptions', value: 'Urgent asks only · until 11:00', detail: 'Only asks with a deadline within an hour reach you.' },
      { label: 'Deadline clock', value: 'Held for routine asks · since 9:45 · until tomorrow 9:00', detail: 'Routine deadlines pause; urgent asks keep running.' },
      { label: 'Note', value: 'Focus on the release.', detail: 'Expires 12:00.' },
    ])
  })
})
