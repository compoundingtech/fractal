import { plainError, type DirectiveNote, type FractalClient, type PersonAvailability } from '@fractal/client'
import { clientReadAuthorization, credentialExpiredMessage } from '../data/source.ts'

/** The session person's own availability and directive note, as Fractal folds them when read. */
export interface OwnAvailability {
  readonly availability: PersonAvailability
  readonly note: DirectiveNote | undefined
}

export type AvailabilityRead =
  | { readonly _tag: 'Observed'; readonly value: OwnAvailability }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

/** Availability is not a followed collection: callers read it when they open, regain focus or reconnect. */
export interface AvailabilityPort {
  readonly read: () => Promise<AvailabilityRead>
}

const unavailable = (detail: string): AvailabilityRead => ({ _tag: 'Unavailable', detail })

/**
 * Reads `GET /v1/client/availability` and `GET /v1/client/notes` without naming a person, so a person
 * session gets its own full view. Only the person's own row carries presence; any other session has none.
 */
export const gatewayAvailability = (client: FractalClient): AvailabilityPort => ({
  read: async () => {
    try {
      const [people, notes] = await Promise.all([client.availabilityList(), client.notesList()])
      const own = people.value.people.find((row) => row.presence !== null)
      if (own === undefined) return unavailable('Availability belongs to a person, and this session is not one.')
      return { _tag: 'Observed', value: { availability: own, note: notes.value.notes.find((note) => note.person === own.person) } }
    } catch (error) {
      const reason = clientReadAuthorization(error)
      if (reason === 'unauthorized') return unavailable(credentialExpiredMessage)
      if (reason === 'ungranted') return unavailable('Reading availability has not been granted to this device.')
      return unavailable(`Availability could not be read: ${plainError(error)}.`)
    }
  },
})
