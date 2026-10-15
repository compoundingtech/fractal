import { St3Client } from '@smalltalk/st3-client'
import { SubjectNote, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import { DateTime } from 'effect'
import { describe, expect, it } from 'vitest'
import { notesByTarget } from './notes.ts'
import { gatewayNoteActions } from './noteActions.ts'

const note = decodeUnknownSync(SubjectNote)({
  id: 'note/person/example/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d',
  author: 'person/example', target: 'agent/example/planner', text: 'Keep the handoff short.', audience: 'self',
  created_at: '2026-10-14T16:20:00Z', updated_at: '2026-10-14T16:20:00Z',
  session: null, expires_at: null, revision: 'claim/example',
})
const makePort = ({ actor = 'person/example', response }: {
  readonly actor?: string
  readonly response?: () => Response
} = {}) => {
  const requests: { path: string; method: string | undefined; body: unknown }[] = []
  const client = new St3Client({
    baseUrl: 'https://gateway.invalid',
    fetchImpl: async (input, init) => {
      requests.push({ path: new URL(String(input)).pathname, method: init?.method, body: JSON.parse(String(init?.body)) })
      return response?.() ?? Response.json({ api_version: 'st3.client.v0', snapshot: {}, value: { id: note.id, note: null } })
    },
  })
  return { actions: gatewayNoteActions({ client, actor: () => actor }), requests }
}

describe('independent subject notes projection and generated writes', () => {
  it('indexes each target newest first without changing the followed rows', () => {
    const older = { ...note, created_at: DateTime.makeUnsafe('2026-10-13T16:20:00Z') }
    const rows = [older, { ...note, target: 'mission/example' }, note]
    const index = notesByTarget(rows)
    expect(index.get(note.target)).toEqual([note, older])
    expect(index.get('mission/example')).toEqual([rows[1]])
    expect(rows[0]).toBe(older)
    expect(notesByTarget([]).size).toBe(0)
  })
  it('creates without an audience and archives using the encoded generated route', async () => {
    const { actions, requests } = makePort()
    await actions.create(note.target, note.text)
    await actions.archive(note)
    expect(requests).toEqual([
      { path: '/v1/subject-notes', method: 'POST', body: { target: note.target, text: note.text } },
      { path: `/v1/subject-notes/${encodeURIComponent(note.id)}`, method: 'PUT', body: { text: null } },
    ])
  })
  it('preserves exact multiline text and target when retrying a refused create', async () => {
    let refuse = true
    const { actions, requests } = makePort({ response: () => refuse
      ? Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'forbidden',
        message: 'Notes control is not granted.', retryable: false, request_id: 'request/example', details: {} }, { status: 403 })
      : Response.json({ api_version: 'st3.client.v0', snapshot: {}, value: { id: note.id, note: null } }) })
    const draft = { target: 'mission/example', text: '  Keep this line\nand this pasted line.  ' }
    await expect(actions.create(draft.target, draft.text)).rejects.toThrow('Notes control is not granted.')
    refuse = false
    await actions.create(draft.target, draft.text)
    expect(requests.map(request => request.body)).toEqual([draft, draft])
  })
  it('sends the same audience-free create for person and agent authors', async () => {
    for (const actor of ['person/example', 'agent/example/planner']) {
      const { actions, requests } = makePort({ actor })
      await actions.create(note.target, note.text)
      expect(requests.map(request => request.body)).toEqual([{ target: note.target, text: note.text }])
    }
  })
  it('refuses creating before the signed-in identity is known', async () => {
    const requests: unknown[] = []
    const client = new St3Client({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input) => { requests.push(input); return Response.json({}) } })
    await expect(gatewayNoteActions({ client, actor: () => undefined }).create(note.target, note.text)).rejects.toThrow('signed-in identity')
    expect(requests).toHaveLength(0)
  })
  it('refuses archiving another author before posting', async () => {
    const { actions, requests } = makePort({ actor: 'agent/example/planner' })
    await expect(actions.archive(note)).rejects.toThrow('your own notes')
    expect(requests).toHaveLength(0)
  })
  it('reports daemon refusals and malformed acknowledgements instead of claiming success', async () => {
    const refused = makePort({ response: () => Response.json({
      api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'forbidden',
      message: 'Notes control is not granted.', retryable: false, request_id: 'request/example', details: {},
    }, { status: 403 }) })
    await expect(refused.actions.create(note.target, note.text)).rejects.toThrow('Notes control is not granted.')
    const malformed = makePort({ response: () => Response.json({ api_version: 'st3.client.v0', snapshot: {}, value: {} }) })
    await expect(malformed.actions.archive(note)).rejects.toThrow('could not be confirmed')
  })
})
