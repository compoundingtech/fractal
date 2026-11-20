// Receipts are daemon-local. A lost reply may only be retried on the original
// member, using its captured transport and fence reader, never the current client.
import { ActionRejected } from './actionReceipt.ts';
import { notApplied, outcomeUnknown } from './errors.ts';

export type DeliveryPorts<TRequest> = {
  send(request: TRequest): Promise<unknown>;
  wait(ms: number): Promise<void>;
  /** An interactive outbox owns when to retry; native immediate sends use bounded retries. */
  retry?: false;
};

export type DeliveryOutcome<TRequest> =
  | { how: 'sent' }
  /** Fractal may have taken it and has not said: `request` is what to send again, unchanged but for its fence. */
  | { how: 'unconfirmed'; request: TRequest }
  | { how: 'failed'; error: unknown };

export type BoundAction<TRequest extends { fence: unknown }> = {
  member: string;
  request: TRequest;
  ports: DeliveryPorts<TRequest> & { fence(): Promise<TRequest['fence']> };
};

export async function sendBoundAction<TRequest extends { fence: unknown }>(
  member: string, ports: DeliveryPorts<TRequest> & { build(): Promise<TRequest>; fence(): Promise<TRequest['fence']> },
): Promise<DeliveryOutcome<BoundAction<TRequest>>> {
  return bindOutcome(await sendAction(ports), member, ports);
}

export async function resendBoundAction<TRequest extends { fence: unknown }>(bound: BoundAction<TRequest>): Promise<DeliveryOutcome<BoundAction<TRequest>>> {
  const outcome = await resendAction(bound.ports, bound.request);
  return outcome.how === 'sent' || outcome.how === 'failed' && outcome.error instanceof ActionRejected ? outcome : { how: 'unconfirmed', request: bound };
}

function bindOutcome<TRequest extends { fence: unknown }>(outcome: DeliveryOutcome<TRequest>, member: string, ports: BoundAction<TRequest>['ports']): DeliveryOutcome<BoundAction<TRequest>> {
  return outcome.how === 'unconfirmed' ? { how: 'unconfirmed', request: { member, request: outcome.request, ports } } : outcome;
}

/** How many requests refusals that applied nothing may cost before the refusal is shown. */
export const SEND_TRIES = 8;
/** How long to wait before sending the identical request again after a lost reply. */
export const REPEAT_AFTER_MS = 1000;

/** Send a new message; `build` makes a request with its own id, key and signature nonce, and a fence. */
export function sendAction<TRequest>(ports: DeliveryPorts<TRequest> & { build(): Promise<TRequest> }): Promise<DeliveryOutcome<TRequest>> {
  return deliver(ports, ports.build);
}

/** Send an unconfirmed message again: the same id, key and parameters, only the fence read anew. */
export function resendAction<TRequest extends { fence: unknown }>(ports: DeliveryPorts<TRequest> & { fence(): Promise<TRequest['fence']> }, request: TRequest): Promise<DeliveryOutcome<TRequest>> {
  return deliver(ports, async () => ({ ...request, fence: await ports.fence() }), request);
}

/** `original` is the unconfirmed request being sent again: whatever Fractal does not answer, it stays unconfirmed. */
async function deliver<TRequest>(ports: DeliveryPorts<TRequest>, next: () => Promise<TRequest>, original?: TRequest): Promise<DeliveryOutcome<TRequest>> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < (ports.retry === false ? 1 : SEND_TRIES); attempt++) {
    if (attempt) { await ports.wait(wait); wait = Math.min(wait * 2, 2000); }
    let request: TRequest;
    try { request = await next(); } catch (error) { return original ? { how: 'unconfirmed', request: original } : { how: 'failed', error }; }
    try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
    if (outcomeUnknown(last)) {
      if (ports.retry === false) return { how: 'unconfirmed', request: original ?? request };
      // Fractal may have taken it: the identical request gets Fractal's first answer, never a second message.
      await ports.wait(REPEAT_AFTER_MS);
      try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
      if (last instanceof ActionRejected) return { how: 'failed', error: last };
      // The first may still land, so a new request could be a second message: keep this one.
      return { how: 'unconfirmed', request: original ?? request };
    }
    if (!notApplied(last)) return original && !(last instanceof ActionRejected)
      ? { how: 'unconfirmed', request: original } : { how: 'failed', error: last };
  }
  return original ? { how: 'unconfirmed', request: original } : { how: 'failed', error: last };
}
