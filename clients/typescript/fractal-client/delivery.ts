// Unknown delivery retains the original member and packet for reconciliation, never replay.
import { notApplied } from './errors.ts';

export type DeliveryPorts<TRequest> = {
  send(request: TRequest): Promise<unknown>;
  wait(ms: number): Promise<void>;
  /** Disable automatic retries even when the response explicitly proves nonapplication. */
  retry?: false;
};

export type DeliveryOutcome<TRequest> =
  | { how: 'sent' }
  /** Dispatch occurred without proof of completion or nonapplication. Retain, but never resend. */
  | { how: 'unconfirmed'; request: TRequest }
  /** Building failed before dispatch, or the server explicitly proved nonapplication. */
  | { how: 'failed'; error: unknown };

export type BoundAction<TRequest extends { fence: unknown }> = {
  member: string;
  request: TRequest;
  ports: DeliveryPorts<TRequest> & { fence(): Promise<TRequest['fence']> };
};

export async function sendBoundAction<TRequest extends { fence: unknown }>(
  member: string, ports: DeliveryPorts<TRequest> & { build(): Promise<TRequest>; fence(): Promise<TRequest['fence']> },
): Promise<DeliveryOutcome<BoundAction<TRequest>>> {
  const outcome = await sendAction(ports);
  return outcome.how === 'unconfirmed' ? { how: 'unconfirmed', request: { member, request: outcome.request, ports } } : outcome;
}

/** Retained unknown attempts cannot be dispatched, even by an explicit resend caller. */
export async function resendBoundAction<TRequest extends { fence: unknown }>(bound: BoundAction<TRequest>): Promise<DeliveryOutcome<BoundAction<TRequest>>> {
  return { how: 'unconfirmed', request: bound };
}

/** How many explicit nonapplication refusals to allow before showing the last refusal. */
export const SEND_TRIES = 8;

/** Build again only after the previous dispatch explicitly proved nonapplication. */
export async function sendAction<TRequest>(ports: DeliveryPorts<TRequest> & { build(): Promise<TRequest> }): Promise<DeliveryOutcome<TRequest>> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < (ports.retry === false ? 1 : SEND_TRIES); attempt++) {
    if (attempt) { await ports.wait(wait); wait = Math.min(wait * 2, 2000); }
    let request: TRequest;
    try { request = await ports.build(); } catch (error) { return { how: 'failed', error }; }
    try { await ports.send(request); return { how: 'sent' }; } catch (error) { last = error; }
    if (!notApplied(last)) return { how: 'unconfirmed', request };
  }
  return { how: 'failed', error: last };
}

/** Keep an unknown request unchanged; neither refresh its fence nor POST it again. */
export async function resendAction<TRequest extends { fence: unknown }>(_ports: DeliveryPorts<TRequest> & { fence(): Promise<TRequest['fence']> }, request: TRequest): Promise<DeliveryOutcome<TRequest>> {
  return { how: 'unconfirmed', request };
}
