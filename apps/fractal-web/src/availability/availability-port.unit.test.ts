import { ClientError, type FractalClient, type PersonAvailability } from '@fractal/client'
import { describe, expect, it } from 'vitest'
import { gatewayAvailability } from './availability-port.ts'

const snapshot = { id: 'snapshot/s1' }
const row = (person: string, presence: PersonAvailability['presence']): PersonAvailability => ({
  person, status: 'available', status_source: 'presence', status_until: null, presence, presence_observed_at: null,
  interruptions: 'all', interruptions_until: null, deadline_clock: 'run', deadline_clock_since: null, deadline_clock_until: null,
})
const note = (person: string) => ({ author: person, person, revision: 'r1', text: `${person} note`, time: '2026-11-12T09:00:00Z' })
const forbidden = (message: string, details: Record<string, unknown> = {}) =>
  new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code: 'forbidden', message, retryable: false, details }, 403)

describe('gatewayAvailability.read', () => {
  it("reads the person's own row and their own note", async () => {
    const client = {
      availabilityList: async () => ({ snapshot, value: { people: [row('person/robin', 'present')] } }),
      notesList: async () => ({ snapshot, value: { notes: [note('person/avery'), note('person/robin')] } }),
    } as unknown as FractalClient
    expect(await gatewayAvailability(client).read()).toEqual({ _tag: 'Observed', value: { availability: row('person/robin', 'present'), note: note('person/robin') } })
  })

  it('says so when the session is no person: no row carries presence', async () => {
    const client = {
      availabilityList: async () => ({ snapshot, value: { people: [row('person/robin', null)] } }),
      notesList: async () => ({ snapshot, value: { notes: [] } }),
    } as unknown as FractalClient
    expect(await gatewayAvailability(client).read()).toEqual({ _tag: 'Unavailable', detail: 'Availability belongs to a person, and this session is not one.' })
  })

  it('tells a missing grant apart from a rejected credential', async () => {
    const refusing = (error: ClientError) => gatewayAvailability({ availabilityList: async () => { throw error }, notesList: async () => ({ snapshot, value: { notes: [] } }) } as unknown as FractalClient)
    expect(await refusing(forbidden('the authenticated client session is not granted `read.projections`')).read())
      .toEqual({ _tag: 'Unavailable', detail: 'Reading availability has not been granted to this device.' })
    expect(await refusing(forbidden('rejected', { reason: 'credential-rejected' })).read())
      .toMatchObject({ _tag: 'Unavailable', detail: expect.stringContaining('Pair the device again') })
  })
})

describe('gatewayAvailability writes', () => {
  it("answers with the state Fractal holds after the write", async () => {
    const away = { ...row('person/robin', 'present'), status: 'away', status_source: 'override' } as const
    const client = {
      availabilitySet: async () => ({ snapshot, value: away }),
      notesSet: async (change: { text: string | null }) => ({ snapshot, value: { note: change.text === null ? null : note('person/robin') } }),
    } as unknown as FractalClient
    const port = gatewayAvailability(client)
    expect(await port.set({ status: { value: 'away' } })).toEqual({ _tag: 'Sent', value: away })
    expect(await port.setNote({ text: 'person/robin note' })).toEqual({ _tag: 'Sent', value: note('person/robin') })
    expect(await port.setNote({ text: null })).toEqual({ _tag: 'Sent', value: undefined })
  })

  it('names re-pairing for a pairing without the write scope, and words any other failure', async () => {
    const failing = (error: Error) => gatewayAvailability({ availabilitySet: async () => { throw error }, notesSet: async () => { throw error } } as unknown as FractalClient)
    const scope = failing(forbidden('the authenticated client session is not granted `write.availability`'))
    expect(await scope.set({ interruptions: { value: 'urgent' } }))
      .toEqual({ _tag: 'Refused', detail: "This device's pairing does not allow changing your availability. Pair it again to change it here." })
    expect(await scope.setNote({ text: null })).toMatchObject({ _tag: 'Refused', detail: expect.stringContaining('changing your note') })
    expect(await failing(forbidden('rejected', { reason: 'credential-rejected' })).set({ status: { value: null } }))
      .toMatchObject({ _tag: 'Refused', detail: expect.stringContaining('Pair the device again') })
    expect(await failing(new TypeError('Failed to fetch')).set({ status: { value: null } }))
      .toEqual({ _tag: 'Refused', detail: 'Your availability was not changed: the connection to Fractal broke (Failed to fetch).' })
  })
})
