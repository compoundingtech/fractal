import { API_VERSION, type ActionResult, type EnvelopeOf } from './Models.generated.ts';
import { ClientError } from './Client.generated.ts';
import { appliedAnswer, outcomeUnknown } from './errors.ts';

/** The durable result each action is allowed to acknowledge; payload and platform auth stay outside it. */
export type ActionReceiptExpectation = { actionId: string } & (
  | { kind: 'message'; targetId?: string; messageId?: string }
  | { kind: 'metadata'; targetId: string }
  | { kind: 'read'; readMessageIds: readonly string[] }
);

const nonempty = (value: unknown): value is string => typeof value === 'string' && value.length > 0;

function unconfirmed(actionId: string): ClientError {
  return new ClientError({
    api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: actionId,
    code: 'internal', message: 'Fractal has not confirmed this action.',
    details: { applied: 'unknown' }, retryable: true,
  }, 502);
}

export class ActionRejected extends Error {
  readonly receipt: ActionResult;
  constructor(receipt: ActionResult) {
    super('The message action was rejected.');
    this.name = 'ActionRejected';
    this.receipt = receipt;
  }
}

/** The HTTP client checks the envelope, but its value still needs the exact action receipt. */
async function confirmActionReceipt<TResponse extends EnvelopeOf<ActionResult>>(send: () => Promise<TResponse>, expected: ActionReceiptExpectation, terminal: boolean): Promise<TResponse> {
  let response: TResponse;
  try {
    response = await send();
  } catch (error) {
    if (appliedAnswer(error) === 'none' || outcomeUnknown(error)
      || error instanceof ClientError && error.status >= 400 && error.status < 500
        && nonempty(error.response.code) && nonempty(error.response.message)) throw error;
    // A truncated or malformed reply can arrive after the daemon has committed the action.
    throw unconfirmed(expected.actionId);
  }
  const value = response?.value;
  if (value?.kind === 'action-result' && value.status === 'rejected' && value.action_id === expected.actionId
    && nonempty(value.operation_id) && nonempty(value.snapshot_id) && Array.isArray(value.affected_ids) && value.affected_ids.every(nonempty)) {
    if (terminal) throw new ActionRejected(value);
    return response;
  }
  if (value?.kind !== 'action-result' || (value.status !== 'completed' && (terminal || value.status !== 'accepted')) || value.action_id !== expected.actionId
    || !nonempty(value.operation_id) || !nonempty(value.snapshot_id)
    || !Array.isArray(value.affected_ids) || !value.affected_ids.every(nonempty)
    || !matchesAffected(value, expected)) {
    throw unconfirmed(expected.actionId);
  }
  return response;
}

function matchesAffected(value: ActionResult, expected: ActionReceiptExpectation): boolean {
  switch (expected.kind) {
    case 'read': return value.affected_ids.every(id => expected.readMessageIds.includes(id));
    case 'metadata': return value.affected_ids.includes(expected.targetId) && nonempty(value.metadata_claim_id);
    case 'message': return (expected.messageId ? value.affected_ids.includes(expected.messageId) : value.affected_ids.some(id => id.startsWith('message/')))
      && (!expected.targetId || value.affected_ids.includes(expected.targetId) && value.affected_ids.length === 2);
  }
}

/** A later refusal cannot resolve an earlier send whose receipt was lost. */
export function actionReceiptCheck(expected: ActionReceiptExpectation, stage: 'terminal' | 'transport' = 'terminal') {
  let missing = false;
  return async <TResponse extends EnvelopeOf<ActionResult>>(send: () => Promise<TResponse>): Promise<TResponse> => {
    try {
      const response = await confirmActionReceipt(send, expected, stage === 'terminal');
      missing = false;
      return response;
    } catch (error) {
      if (missing && !(error instanceof ActionRejected)) throw unconfirmed(expected.actionId);
      missing = outcomeUnknown(error);
      throw error;
    }
  };
}
