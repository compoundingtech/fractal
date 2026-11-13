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

describe('gatewayAvailability.reportPresence', () => {
  it('answers with report_again_at, and tells a refusal apart from a failed report', async () => {
    const report = { state: 'active', last_input_at: '2026-11-12T09:58:00Z', observed_at: '2026-11-12T09:58:30Z' } as const
    const answering = (reportAgainAt: string | null) => gatewayAvailability({
      presenceReport: async () => ({ snapshot, value: { availability: row('person/robin', 'present'), report_again_at: reportAgainAt } }),
    } as unknown as FractalClient)
    expect(await answering('2026-11-12T10:08:00Z').reportPresence(report)).toEqual({ _tag: 'Sent', reportAgainAt: '2026-11-12T10:08:00Z' })
    expect(await answering(null).reportPresence(report)).toEqual({ _tag: 'Sent', reportAgainAt: null })
    const failing = (error: Error) => gatewayAvailability({ presenceReport: async () => { throw error } } as unknown as FractalClient)
    expect(await failing(forbidden('the authenticated client session is not granted `write.presence`')).reportPresence(report)).toEqual({ _tag: 'Refused' })
    // A daemon without the route refuses for good too, rather than being retried for the page's life.
    const missing = new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code: 'not-found', message: 'no route', retryable: false, details: {} }, 404)
    expect(await failing(missing).reportPresence(report)).toEqual({ _tag: 'Refused' })
    expect(await failing(new TypeError('Failed to fetch')).reportPresence(report)).toEqual({ _tag: 'Failed' })
  })
})

describe('gatewayAvailability.mayReportPresence', () => {
  const capabilities = (session_actor: string, state: string) => gatewayAvailability({
    capabilities: async () => ({ snapshot, value: { session_actor, capabilities: [{ id: 'read.projections', state: 'granted', version: 1 }, { id: 'write.presence', state, version: 1 }] } }),
  } as unknown as FractalClient)

  it('allows a person session whose capabilities grant write.presence', async () => {
    expect(await capabilities('person/robin', 'granted').mayReportPresence()).toBe(true)
    expect(await capabilities('person/robin', 'ungranted').mayReportPresence()).toBe(false)
    expect(await capabilities('agent/robin-helper', 'granted').mayReportPresence()).toBe(false)
    expect(await gatewayAvailability({ capabilities: async () => { throw new TypeError('Failed to fetch') } } as unknown as FractalClient).mayReportPresence()).toBe(false)
  })
})
