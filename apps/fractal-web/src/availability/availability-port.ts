import { ClientError, plainError, type AvailabilitySet, type DirectiveNote, type DirectiveNoteSet, type FractalClient, type PersonAvailability, type PresenceReport } from '@fractal/client'
import { clientReadAuthorization, credentialExpiredMessage } from '../data/source.ts'

/** The session person's own availability and directive note, as Fractal folds them when read. */
export interface OwnAvailability {
  readonly availability: PersonAvailability
  readonly note: DirectiveNote | undefined
}

export type AvailabilityRead =
  | { readonly _tag: 'Observed'; readonly value: OwnAvailability }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

/** A write's answer: the state Fractal holds after it, or why it was refused, in words a person reads. */
export type AvailabilityWrite<A> = { readonly _tag: 'Sent'; readonly value: A } | { readonly _tag: 'Refused'; readonly detail: string }

/**
 * A presence report's answer. `Sent` carries `report_again_at`: when to report again if nothing changes, or null for
 * the next change or the first input after an idle stretch. `Refused`: this session may not report (403: no
 * `write.presence` grant, or no person; 404: a daemon without the route). `Failed`: the report did not arrive.
 */
export type PresenceAnswer =
  | { readonly _tag: 'Sent'; readonly reportAgainAt: string | null }
  | { readonly _tag: 'Refused' }
  | { readonly _tag: 'Failed' }

/** Availability is not a followed collection: callers read it when they open, regain focus or reconnect, and after their own writes. */
export interface AvailabilityPort {
  readonly read: () => Promise<AvailabilityRead>
  /** `PUT /v1/client/availability`: an absent axis is unchanged; a null status follows presence again. */
  readonly set: (change: AvailabilitySet) => Promise<AvailabilityWrite<PersonAvailability>>
  /** `PUT /v1/client/notes`: null text clears the note; the answer is the note now in force, if any. */
  readonly setNote: (change: DirectiveNoteSet) => Promise<AvailabilityWrite<DirectiveNote | undefined>>
  /** Whether this session may report presence: its capabilities grant `write.presence` and its actor is a person. */
  readonly mayReportPresence: () => Promise<boolean>
  /** `POST /v1/client/presence` with the device's raw state; the daemon derives presence and assigns the source. */
  readonly reportPresence: (report: PresenceReport) => Promise<PresenceAnswer>
}

const unavailable = (detail: string): AvailabilityRead => ({ _tag: 'Unavailable', detail })

/** Only a person's session sees the editor, so a 403 is a pairing without the write scope or a rejected credential. */
const refused = (error: unknown, what: string): { readonly _tag: 'Refused'; readonly detail: string } => ({
  _tag: 'Refused',
  detail: clientReadAuthorization(error) === 'unauthorized' ? credentialExpiredMessage
    : error instanceof ClientError && error.status === 403 ? `This device's pairing does not allow changing your ${what}. Pair it again to change it here.`
      : `Your ${what} was not changed: ${plainError(error)}.`,
})
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
  set: async (change) => {
    try {
      return { _tag: 'Sent', value: (await client.availabilitySet(change)).value }
    } catch (error) {
      return refused(error, 'availability')
    }
  },
  setNote: async (change) => {
    try {
      return { _tag: 'Sent', value: (await client.notesSet(change)).value.note ?? undefined }
    } catch (error) {
      return refused(error, 'note')
    }
  },
  // A capabilities read that fails counts as no grant: the page reports nothing until it is loaded again.
  mayReportPresence: async () => {
    try {
      const { value } = await client.capabilities()
      return value.session_actor.startsWith('person/') && value.capabilities.some((capability) => capability.id === 'write.presence' && capability.state === 'granted')
    } catch {
      return false
    }
  },
  reportPresence: async (report) => {
    try {
      return { _tag: 'Sent', reportAgainAt: (await client.presenceReport(report)).value.report_again_at }
    } catch (error) {
      return error instanceof ClientError && (error.status === 403 || error.status === 404) ? { _tag: 'Refused' } : { _tag: 'Failed' }
    }
  },
})
