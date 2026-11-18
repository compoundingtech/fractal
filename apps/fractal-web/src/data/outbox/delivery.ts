import { actionReceiptCheck, appliedAnswer, type ActionReceiptExpectation, type FractalClient } from '@fractal/client'
import { observeOutbox, type OutboxObservation, type OutboxRecord } from '@fractal/client/outbox'
import type { PreparedPacket } from '@fractal/client/prepared-packet'
import type { ActionResult } from '@fractal/client/schema'
import type { IndexedDbOutbox } from './indexedDb.ts'
import { sameScope } from './repository.ts'
import { scopeFromCapabilities } from './scoped.ts'

export type OutboxDeliveryOutcome =
  | { readonly how: 'sent'; readonly receipt: ActionResult }
  | { readonly how: 'unconfirmed' }
  | { readonly how: 'not-applied'; readonly error: unknown }

export class OutboxScopeMismatch extends Error {}
export const requireOutboxScope = async (client: FractalClient, repository: IndexedDbOutbox): Promise<void> => {
  const scope = scopeFromCapabilities(await client.capabilities())
  if (scope === undefined || !sameScope(repository.scope, scope)) {
    throw new OutboxScopeMismatch('Return to the original account and deployment to inspect or send this action.')
  }
}

/** The transaction winner alone sends; dismissed outcomes stop without retrying writes. */
export const dispatchOutbox = async (
  client: FractalClient,
  repository: IndexedDbOutbox,
  record: OutboxRecord<PreparedPacket>,
  expected: ActionReceiptExpectation,
  decodeReceipt: (value: unknown) => ActionResult,
): Promise<OutboxDeliveryOutcome> => {
  await requireOutboxScope(client, repository)
  let retained: OutboxRecord<PreparedPacket>
  try { retained = await repository.acquireDispatchPermit(record.id, record.revision) }
  catch { return { how: 'unconfirmed' } }
  if (!('packet' in retained)) return { how: 'unconfirmed' }
  const retainObservation = async (observation: OutboxObservation): Promise<OutboxRecord<PreparedPacket> | undefined> => {
    let current: OutboxRecord<PreparedPacket> | undefined = retained
    while (current !== undefined) {
      await requireOutboxScope(client, repository)
      const outcome = observeOutbox(current, observation)
      if (outcome === current) return current
      const written = await repository.compareAndSwapWritable(current.revision, outcome)
      if (written._tag === 'written') return written.record
      if (written._tag === 'not-writable') return undefined
      current = await repository.read(record.id)
    }
    return undefined
  }
  try {
    const packet = retained.packet
    const response = await actionReceiptCheck(expected)(() => client.submitAction(packet))
    const receipt = decodeReceipt(response.value)
    if (response.value.status !== 'completed') return { how: 'unconfirmed' }
    const outcome = await retainObservation({ kind: 'delivered', receipt: { ...response.value, status: 'completed' } })
    return outcome?.state === 'delivered' ? { how: 'sent', receipt } : { how: 'unconfirmed' }
  } catch (error) {
    let outcome: OutboxRecord<PreparedPacket> | undefined
    try { outcome = await retainObservation({ kind: 'failure', applied: appliedAnswer(error) }) }
    catch { return { how: 'unconfirmed' } }
    return outcome?.state === 'not-applied' ? { how: 'not-applied', error } : { how: 'unconfirmed' }
  }
}
