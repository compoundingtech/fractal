import { ClientError } from '@fractal/client'
import { expect, it } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { unreadReceiptCheck } from './unreadReceipt.ts'

const receipt = (affected_ids: string[]) => ({ ...fixture.capabilities, value: {
  kind: 'action-result' as const, action_id: 'action/unread', operation_id: 'operation/transport-only',
  status: 'completed' as const, affected_ids, snapshot_id: fixture.capabilities.snapshot.id,
} })

it('confirms the exact thread without a metadata claim and does not synthesize mark IDs', async () => {
  const response = receipt(['thread/a'])
  expect(await unreadReceiptCheck({ actionId: 'action/unread', targetId: 'thread/a' })(async () => response)).toBe(response)
  expect(response.value).not.toHaveProperty('mark_id')
})
it.each([[], ['thread/other'], ['thread/a', 'message/extra']].map(affected => ({ affected })))('refuses a mismatched affected thread set: $affected', async ({ affected }) => {
  await expect(unreadReceiptCheck({ actionId: 'action/unread', targetId: 'thread/a' })(async () => receipt(affected))).rejects.toBeInstanceOf(ClientError)
})
it('does not acknowledge another action or an unconfirmed operation receipt', async () => {
  const check = unreadReceiptCheck({ actionId: 'action/other', targetId: 'thread/a' })
  await expect(check(async () => receipt(['thread/a']))).rejects.toBeInstanceOf(ClientError)
  await expect(unreadReceiptCheck({ actionId: 'action/unread', targetId: 'thread/a' })(async () => ({ ...receipt(['thread/a']), value: { ...receipt(['thread/a']).value, operation_id: '' } }))).rejects.toBeInstanceOf(ClientError)
})
