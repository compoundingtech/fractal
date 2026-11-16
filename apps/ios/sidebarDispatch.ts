import { ClientError, type ActionRequest, type ActionResult, type FractalClient } from '../../clients/typescript/fractal-client';
import type { WriteDelivery } from './sidebarJournal';
import type { WriteEvent, WriteRequest } from './sidebarWriteState';

/** Call only for a journal Send effect, with the actor of its authenticated connection. */
export const sidebarWriteRequest = (request: WriteRequest): ActionRequest => {
  const { identity, fence, operations } = request;
  const common = { api_version: 'fractal.client.v0' as const, id: identity.action_id, idempotency_key: identity.idempotency_key,
    fence: { snapshot_id: fence.snapshot.id, subject_revisions: { [fence.scope.container]: fence.revision } } };
  const target = { subject: fence.scope.container, owner: fence.scope.owner };
  return 'Folders' in operations
    ? { ...common, type: 'arrangement.edit', parameters: { ...target, operations: structuredClone(operations.Folders) } }
    : { ...common, type: 'arrangement.membership.edit', parameters: { ...target, operations: structuredClone(operations.Memberships) } };
};
/** A receipt acknowledges delivery, never authoritative observation. Unknown means replay exactly. */
export const sidebarWriteResult = (request: WriteRequest, delivery: WriteDelivery, result: { value: ActionResult } | { error: unknown }): WriteEvent => {
  const identity = structuredClone(request.identity);
  if ('value' in result) {
    if (result.value.action_id !== identity.action_id) return { type: 'Unknown', identity };
    switch (result.value.status) {
      case 'accepted': case 'completed': return { type: 'Ack', identity };
      case 'rejected': return { type: 'Refused', identity, reason: 'Conflict' };
      default: return { type: 'Unknown', identity };
    }
  }
  // Admission may reject a replay before looking up an earlier committed receipt.
  if (delivery === 'Replay') return { type: 'Unknown', identity };
  if (!(result.error instanceof ClientError)) return { type: 'Unknown', identity };
  const response = result.error.response, applied = response.applied ?? response.details.applied;
  if (applied === 'unknown') return { type: 'Unknown', identity };
  let reason: 'StaleFence' | 'Unauthorized' | 'DestinationMissing' | 'InvalidPlan' | 'Conflict' | undefined;
  switch (response.code) {
    case 'stale-fence': reason = 'StaleFence'; break;
    case 'forbidden': case 'arrangement-owner-forbidden': case 'membership-owner-forbidden': reason = 'Unauthorized'; break;
    case 'arrangement-folder-deleted': case 'arrangement-retired': case 'not-found': reason = 'DestinationMissing'; break;
    case 'invalid-membership-operations': reason = 'InvalidPlan'; break;
    case 'idempotency-conflict': if (applied === 'none') reason = 'Conflict'; break;
    default: if (applied === 'none') reason = 'InvalidPlan';
  }
  return reason ? { type: 'Refused', identity, reason } : { type: 'Unknown', identity };
};
export const sendSidebarWrite = async (client: Pick<FractalClient, 'submitAction'>, request: WriteRequest, delivery: WriteDelivery, actor: string, signal: AbortSignal): Promise<WriteEvent> => {
  if (actor !== request.identity.actor) return delivery === 'Replay' ? { type: 'Unknown', identity: structuredClone(request.identity) } : { type: 'Refused', identity: structuredClone(request.identity), reason: 'Unauthorized' };
  try { return sidebarWriteResult(request, delivery, await client.submitAction(sidebarWriteRequest(request), { signal })); }
  catch (error) { return sidebarWriteResult(request, delivery, { error }); }
};
