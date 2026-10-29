// Receipts are daemon-local. A lost reply may only be retried on the original
// member, using its captured transport and fence reader, never the current client.
import { notApplied, outcomeUnknown } from '../../clients/typescript/fractal-client';

export type SendPorts<R> = {
  send(request: R): Promise<unknown>;
  wait(ms: number): Promise<void>;
};

export type SendOutcome<R> =
  | { how: 'sent' }
  /** Fractal may have taken it and has not said: `request` is what to send again, unchanged but for its fence. */
  | { how: 'unconfirmed'; request: R }
  | { how: 'failed'; error: unknown };

export type BoundMessage<R extends { fence: unknown }> = {
  member: string;
  request: R;
  ports: SendPorts<R> & { fence(): Promise<R['fence']> };
};

export async function sendBoundMessage<R extends { fence: unknown }>(
  member: string, ports: SendPorts<R> & { build(): Promise<R>; fence(): Promise<R['fence']> },
): Promise<SendOutcome<BoundMessage<R>>> {
  return bindOutcome(await sendMessage(ports), member, ports);
}

export async function resendBoundMessage<R extends { fence: unknown }>(bound: BoundMessage<R>): Promise<SendOutcome<BoundMessage<R>>> {
  const outcome = await resendMessage(bound.ports, bound.request);
  return outcome.how === 'sent' ? outcome : { how: 'unconfirmed', request: bound };
}

function bindOutcome<R extends { fence: unknown }>(outcome: SendOutcome<R>, member: string, ports: BoundMessage<R>['ports']): SendOutcome<BoundMessage<R>> {
  return outcome.how === 'unconfirmed' ? { how: 'unconfirmed', request: { member, request: outcome.request, ports } } : outcome;
}

/** How many requests refusals that applied nothing may cost before the refusal is shown. */
export const SEND_TRIES = 8;
/** How long to wait before sending the identical request again after a lost reply. */
export const REPEAT_AFTER_MS = 1000;

/** Send a new message; `build` makes a request with its own id, key and signature nonce, and a fence. */
export function sendMessage<R>(ports: SendPorts<R> & { build(): Promise<R> }): Promise<SendOutcome<R>> {
  return deliver(ports, ports.build);
}

/** Send an unconfirmed message again: the same id, key and parameters, only the fence read anew. */
export function resendMessage<R extends { fence: unknown }>(ports: SendPorts<R> & { fence(): Promise<R['fence']> }, request: R): Promise<SendOutcome<R>> {
  return deliver(ports, async () => ({ ...request, fence: await ports.fence() }), request);
}

/** `original` is the unconfirmed request being sent again: whatever Fractal does not answer, it stays unconfirmed. */
async function deliver<R>(ports: SendPorts<R>, next: () => Promise<R>, original?: R): Promise<SendOutcome<R>> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < SEND_TRIES; attempt++) {
    if (attempt) { await ports.wait(wait); wait = Math.min(wait * 2, 2000); }
    let request: R;
    try { request = await next(); } catch (error) { return original ? { how: 'unconfirmed', request: original } : { how: 'failed', error }; }
    try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
    if (outcomeUnknown(last)) {
      // Fractal may have taken it: the identical request gets Fractal's first answer, never a second message.
      await ports.wait(REPEAT_AFTER_MS);
      try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
      // The first may still land, so a new request could be a second message: keep this one.
      if (outcomeUnknown(last) || notApplied(last)) return { how: 'unconfirmed', request: original ?? request };
    }
    if (!notApplied(last)) return { how: 'failed', error: last };
  }
  return original ? { how: 'unconfirmed', request: original } : { how: 'failed', error: last };
}
