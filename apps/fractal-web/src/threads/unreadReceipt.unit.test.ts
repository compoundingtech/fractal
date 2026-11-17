import { actionReceiptCheck, ClientError, type ActionResult, type EnvelopeOf } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Schema } from 'effect'
import { expect, it } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'

const envelope = Schema.encodeSync(Native.Envelope)(Native.decodeUnknownSync(Native.Envelope)(fixture.capabilities))
const receipt = (affected_ids: string[]): EnvelopeOf<ActionResult> => {
  const value = Schema.encodeSync(Native.ActionResult)(Native.decodeUnknownSync(Native.ActionResult)({
    kind: 'action-result', action_id: 'action/unread', operation_id: 'operation/transport-only',
    status: 'completed', affected_ids, snapshot_id: fixture.capabilities.snapshot.id,
  }))
  return { ...envelope, snapshot: { ...envelope.snapshot, published_at: envelope.snapshot.published_at ?? undefined }, value: {
    kind: value.kind, action_id: value.action_id, operation_id: value.operation_id,
    status: value.status, affected_ids: [...value.affected_ids], snapshot_id: value.snapshot_id,
  } }
}

it('confirms the exact thread without a metadata claim and does not synthesize mark IDs', async () => {
  const response = receipt(['thread/a'])
  expect(await actionReceiptCheck({ kind: 'unread', actionId: 'action/unread', targetId: 'thread/a' })(async () => response)).toBe(response)
  expect(response.value).not.toHaveProperty('mark_id')
})
it.each([[], ['thread/other'], ['thread/a', 'message/extra']].map(affected => ({ affected })))('refuses a mismatched affected thread set: $affected', async ({ affected }) => {
  await expect(actionReceiptCheck({ kind: 'unread', actionId: 'action/unread', targetId: 'thread/a' })(async () => receipt(affected))).rejects.toBeInstanceOf(ClientError)
})
it('does not acknowledge another action or an unconfirmed operation receipt', async () => {
  const check = actionReceiptCheck({ kind: 'unread', actionId: 'action/other', targetId: 'thread/a' })
  await expect(check(async () => receipt(['thread/a']))).rejects.toBeInstanceOf(ClientError)
  await expect(actionReceiptCheck({ kind: 'unread', actionId: 'action/unread', targetId: 'thread/a' })(async () => ({ ...receipt(['thread/a']), value: { ...receipt(['thread/a']).value, operation_id: '' } }))).rejects.toBeInstanceOf(ClientError)
})
