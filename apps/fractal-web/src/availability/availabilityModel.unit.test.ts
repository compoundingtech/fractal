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
      { label: 'Interruptions', value: 'Urgent asks only · until 11:00', detail: 'Only asks with priority critical reach you.' },
      { label: 'Deadline clock', value: 'Held for routine asks · since 9:45 · until tomorrow 9:00', detail: 'Routine deadlines pause; urgent asks keep running.' },
      { label: 'Note', value: 'Focus on the release.', detail: 'Expires 12:00.' },
    ])
  })

  it('shows an unreported person as available with no device reporting, never as away', () => {
    const unreported: PersonAvailability = { ...base, status_source: 'unreported', presence: 'unreported', presence_observed_at: null }
    expect(availabilityFacts({ availability: unreported, note: undefined }, now, zone)[0])
      .toEqual({ label: 'Status', value: 'Available', detail: 'No device is reporting your presence, so agents ask as usual.' })
    // An override still names presence; an unreported one reads as not reported, without a stale time.
    const override: PersonAvailability = { ...base, status: 'away', status_source: 'override', presence: 'unreported', presence_observed_at: iso(-3000) }
    expect(availabilityFacts({ availability: override, note: undefined }, now, zone)[0])
      .toEqual({ label: 'Status', value: 'Away', detail: 'Set by you; presence is not reported.' })
  })

  it('shows words this build does not know as themselves', () => {
    const future: PersonAvailability = {
      ...base, status: 'heads-down', status_source: 'calendar', presence: 'driving',
      interruptions: 'family-only', deadline_clock: 'hold-weekend', deadline_clock_since: iso(-15),
    }
    expect(availabilityFacts({ availability: future, note: undefined }, now, zone)).toEqual([
      { label: 'Status', value: 'heads-down', detail: 'Comes from calendar; presence is driving 2m ago.' },
      { label: 'Interruptions', value: 'family-only' },
      { label: 'Deadline clock', value: 'hold-weekend · since 9:45' },
      { label: 'Note', value: 'No note' },
    ])
  })
})
