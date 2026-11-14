/** Literal contract fixtures for the availability panel, anchored to the fixture world's clock. */
import type { AvailabilitySet, DirectiveNote, PersonAvailability } from '@fractal/client'
import { worldNow } from '../fixtures/world.ts'
import type { AvailabilityPort, AvailabilityRead, OwnAvailability } from './availability-port.ts'

const minutes = (count: number) => new Date(worldNow + count * 60_000).toISOString()
const person = 'person/robin'

const presenceAvailable: PersonAvailability = {
  person, status: 'available', status_source: 'presence', status_until: null, presence: 'present', presence_observed_at: minutes(-3),
  interruptions: 'all', interruptions_until: null, deadline_clock: 'run', deadline_clock_since: null, deadline_clock_until: null,
}
const note: DirectiveNote = { author: person, person, revision: 'note/r1', text: 'Ship the release notes first.', time: minutes(-40), expires_at: minutes(180) }

export const availabilityFixtures = {
  available: { availability: presenceAvailable, note: undefined },
  away: {
    availability: {
      ...presenceAvailable, status: 'away', status_source: 'override', status_until: minutes(90), presence: 'idle',
      interruptions: 'urgent', interruptions_until: minutes(60), deadline_clock: 'hold-routine', deadline_clock_since: minutes(-15), deadline_clock_until: minutes(240),
    },
    note,
  },
  urgent: { availability: { ...presenceAvailable, interruptions: 'urgent', interruptions_until: minutes(120) }, note: undefined },
  holdRoutine: { availability: { ...presenceAvailable, deadline_clock: 'hold-routine', deadline_clock_since: minutes(-30), deadline_clock_until: minutes(180) }, note },
} satisfies Readonly<Record<string, OwnAvailability>>

export type AvailabilityFixture = keyof typeof availabilityFixtures

export const observedAvailability = (state: AvailabilityFixture): AvailabilityRead => ({ _tag: 'Observed', value: availabilityFixtures[state] })

const at = new Date(worldNow).toISOString()
const fold = (a: PersonAvailability, { status, interruptions, deadline_clock: clock }: AvailabilitySet): PersonAvailability => ({
  ...a,
  // Following presence again: available while present, away while devices report but none is present, and the
  // default available when no device reports.
  ...(status === undefined ? {} : status.value === null
    ? a.presence === 'unreported' || a.presence === null
      ? { status: 'available', status_source: 'unreported', status_until: null }
      : { status: a.presence === 'present' ? 'available' : 'away', status_source: 'presence', status_until: null }
    : { status: status.value, status_source: 'override', status_until: status.until ?? null }),
  ...(interruptions === undefined ? {} : { interruptions: interruptions.value, interruptions_until: interruptions.until ?? null }),
  ...(clock === undefined ? {} : { deadline_clock: clock.value, deadline_clock_since: clock.value === 'run' ? null : at, deadline_clock_until: clock.until ?? null }),
})

/** A story port over one fixture that answers writes as Fractal folds them, or refuses them as a pairing without the write scopes. */
export const fixtureAvailabilityPort = (state: AvailabilityFixture, { refuse = false } = {}): AvailabilityPort => {
  let current: OwnAvailability = availabilityFixtures[state]
  let revision = 1
  const refused = { _tag: 'Refused', detail: "This device's pairing does not allow changing your availability. Pair it again to change it here." } as const
  return {
    read: async () => ({ _tag: 'Observed', value: current }),
    set: async (change) => {
      if (refuse) return refused
      current = { ...current, availability: fold(current.availability, change) }
      return { _tag: 'Sent', value: current.availability }
    },
    setNote: async ({ text, expires_at }) => {
      if (refuse) return refused
      current = { ...current, note: text === null ? undefined : { author: person, person, revision: `note/r${++revision}`, text, time: at, expires_at: expires_at ?? null } }
      return { _tag: 'Sent', value: current.note }
    },
  }
}
