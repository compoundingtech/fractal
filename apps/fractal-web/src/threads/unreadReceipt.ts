import { actionReceiptCheck as sharedActionReceiptCheck, ClientError, type ActionReceiptExpectation, type ActionResult, type EnvelopeOf } from '@fractal/client'

/** Web-local compatibility seam; replace this import with the shared helper once it accepts unread receipts. */
export const actionReceiptCheck = (expected: ActionReceiptExpectation | {
  readonly kind: 'unread'; readonly actionId: string; readonly targetId: string
}, stage: 'terminal' | 'transport' = 'terminal') => {
  if (expected.kind !== 'unread') return sharedActionReceiptCheck(expected, stage)
  const { actionId, targetId } = expected
  // Shared validation owns the envelope/action/status/unknown-outcome behavior; unread requires the exact thread.
  const check = sharedActionReceiptCheck({ kind: 'read', actionId, readMessageIds: [targetId] }, stage)
  return <TResponse extends EnvelopeOf<ActionResult>>(send: () => Promise<TResponse>): Promise<TResponse> => check(async () => {
    const response = await send()
    const value = response.value
    if (value.status !== 'rejected' && (value.status !== 'completed' || value.affected_ids.length !== 1 || value.affected_ids[0] !== targetId)) {
      throw new ClientError({
        api_version: response.api_version, error_version: 'st3.client.error.v0', request_id: actionId,
        code: 'internal', message: 'Fractal has not confirmed this unread reminder change.',
        details: { applied: 'unknown' }, retryable: true,
      }, 502)
    }
    // operation_id confirms transport only. Mark IDs come exclusively from Summary.unread_reminder.
    return response
  })
}
