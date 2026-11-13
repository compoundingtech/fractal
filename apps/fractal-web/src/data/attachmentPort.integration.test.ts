import { FractalClient } from '@fractal/client'
import { expect, it } from 'vitest'
import fixture from './subjectReadPort.gateway.fixtures.json'
import { gatewayMessageSend } from './attachmentPort.ts'
import type { MessageSendAction } from './source.ts'

it('keeps a committed message unconfirmed when decoding its receipt fails', async () => {
  const base = fixture.capabilities
  const writes: unknown[] = []
  const request: MessageSendAction = {
    api_version: 'st3.client.v0', type: 'message.send', id: 'action/attachment-receipt', idempotency_key: 'attachment-receipt',
    fence: { snapshot_id: base.snapshot.id, subject_revisions: {} },
    parameters: { to: 'agent/atlas', content: 'An attached message', tags: [], attachments: [] },
  }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(request.idempotency_key)))
  const message = `message/${[...digest.slice(0, 8)].map(byte => byte.toString(16).padStart(2, '0')).join('')}`
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return Response.json(base)
    writes.push(JSON.parse(String(init?.body)))
    return Response.json({ ...base, value: { kind: 'action-result', status: 'completed', action_id: request.id,
      operation_id: 'operation/send', snapshot_id: base.snapshot.id, affected_ids: [message],
      ...(writes.length === 1 ? { terminal_attachment: { state: 'malformed' } } : {}),
    } })
  } })
  const send = gatewayMessageSend(client)
  expect(await send(request)).toMatchObject({ _tag: 'Refused', reason: 'unconfirmed' })
  expect(await send(request)).toMatchObject({ _tag: 'Success', value: { action_id: request.id } })
  expect(writes).toHaveLength(2)
  expect(writes[1]).toEqual(writes[0])
})
