// A message st may have taken stays that one message until st says what became of it.
//
// A message is sent once with its own id and key, signed with a fresh nonce. When st's reply is
// lost, or st says it may still complete (`applied: "unknown"`, as `message-send-unconfirmed` does
// when it cannot confirm in time), the identical request is sent again: st returns its first answer
// and never records a second message. If st still has not said, the send is unconfirmed and the
// app keeps the exact request, so the person can send it again: the same id, key and parameters
// (signature and nonce included) under a fresh fence. A request rebuilt with a new id could be a
// second message; a changed one under the same key is refused (`idempotency-conflict`).
//
// Only a refusal that guarantees nothing was applied (a stale fence, `rate-limited`,
// `applied: "none"`) is tried again with a new request.
import { notApplied, outcomeUnknown } from '../../clients/typescript/fractal-client';

export type SendPorts<R> = {
  send(request: R): Promise<unknown>;
  wait(ms: number): Promise<void>;
};

export type SendOutcome<R> =
  | { how: 'sent' }
  /** st may have taken it and has not said: `request` is what to send again, unchanged but for its fence. */
  | { how: 'unconfirmed'; request: R }
  | { how: 'failed'; error: unknown };

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

/** `original` is the unconfirmed request being sent again: whatever st does not answer, it stays unconfirmed. */
async function deliver<R>(ports: SendPorts<R>, next: () => Promise<R>, original?: R): Promise<SendOutcome<R>> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < SEND_TRIES; attempt++) {
    if (attempt) { await ports.wait(wait); wait = Math.min(wait * 2, 2000); }
    let request: R;
    try { request = await next(); } catch (error) { return original ? { how: 'unconfirmed', request: original } : { how: 'failed', error }; }
    try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
    if (outcomeUnknown(last)) {
      // st may have taken it: the identical request gets st's first answer, never a second message.
      await ports.wait(REPEAT_AFTER_MS);
      try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
      // The first may still land, so a new request could be a second message: keep this one.
      if (outcomeUnknown(last) || notApplied(last)) return { how: 'unconfirmed', request: original ?? request };
    }
    if (!notApplied(last)) return { how: 'failed', error: last };
  }
  return original ? { how: 'unconfirmed', request: original } : { how: 'failed', error: last };
}
