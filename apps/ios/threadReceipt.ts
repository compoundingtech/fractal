import { API_VERSION, ClientError, appliedAnswer, outcomeUnknown, type ActionResult, type EnvelopeOf } from '../../clients/typescript/fractal-client';

type ExpectedReceipt = { actionId: string; targetId: string; messageId?: string };
const nonempty = (value: unknown): value is string => typeof value === 'string' && value.length > 0;

function unconfirmed(actionId: string): ClientError {
  return new ClientError({
    api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: actionId,
    code: 'internal', message: 'Fractal has not confirmed this thread action.',
    details: { applied: 'unknown' }, retryable: true,
  }, 502);
}

/** The HTTP client checks the envelope, but its value still needs the exact action receipt. */
async function confirmThreadReceipt(send: () => Promise<EnvelopeOf<ActionResult>>, expected: ExpectedReceipt): Promise<void> {
  let response: EnvelopeOf<ActionResult>;
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
  if (value?.kind !== 'action-result' || value.status !== 'completed' || value.action_id !== expected.actionId
    || !nonempty(value.operation_id) || !nonempty(value.snapshot_id)
    || !Array.isArray(value.affected_ids) || !value.affected_ids.every(nonempty) || !value.affected_ids.includes(expected.targetId)
    || !(expected.messageId ? value.affected_ids.includes(expected.messageId) : nonempty(value.metadata_revision))) {
    throw unconfirmed(expected.actionId);
  }
}

/** A later refusal cannot resolve an earlier send whose receipt was lost. */
export function threadReceiptCheck(expected: ExpectedReceipt) {
  let missing = false;
  return async (send: () => Promise<EnvelopeOf<ActionResult>>): Promise<void> => {
    try {
      await confirmThreadReceipt(send, expected);
      missing = false;
    } catch (error) {
      if (missing) throw unconfirmed(expected.actionId);
      missing = outcomeUnknown(error);
      throw error;
    }
  };
}
