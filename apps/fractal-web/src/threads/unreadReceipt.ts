import { actionReceiptCheck, ClientError, type ActionResult, type EnvelopeOf } from '@fractal/client'

/** Unread actions acknowledge exactly the thread, without a metadata claim or a message-read receipt. */
export const unreadReceiptCheck = ({ actionId, targetId }: { readonly actionId: string; readonly targetId: string }) => {
  // Reuse envelope/action/status/unknown-outcome handling; the allowed affected set is checked more strictly below.
  const check = actionReceiptCheck({ kind: 'read', actionId, readMessageIds: [targetId] })
  return <TResponse extends EnvelopeOf<ActionResult>>(send: () => Promise<TResponse>): Promise<TResponse> => check(async () => {
    const response = await send()
    const value = response.value
    if (value.status !== 'rejected' && (value.affected_ids.length !== 1 || value.affected_ids[0] !== targetId)) {
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
