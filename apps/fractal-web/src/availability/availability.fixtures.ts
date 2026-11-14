/** Literal contract fixtures for the availability panel, anchored to the fixture world's clock. */
import type { DirectiveNote, PersonAvailability } from '@fractal/client'
import { worldNow } from '../fixtures/world.ts'
import type { AvailabilityRead, OwnAvailability } from './availability-port.ts'

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
