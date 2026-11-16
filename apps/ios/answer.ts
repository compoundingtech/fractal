// Answering something that waits on the person (a decision, an ask, a gate) must end in what the
// person would call true: if the answer landed, the app says so, however the reply came back.
//
// An answer is sent once with its own key. When Fractal's reply is lost, or Fractal says it may still
// complete (`applied: "unknown"`), the identical request is sent again (the same key, so Fractal
// returns its first answer and never records a second). When Fractal refuses with a stale fence, or the
// reply never arrives for good, the app asks whether the thing still waits: if it does not, the
// answer is done (this one landed, or someone answered it first) and a repeat submit is a success,
// not a failure. A refusal while it still waits is shown as the refusal it is.
import { appliedAnswer, ClientError, notApplied, outcomeUnknown } from '../../clients/typescript/fractal-client';

export type AnswerPorts<R> = {
  /** A new request with its own key, fenced to an admissible held snapshot or a fresh read. */
  build(): Promise<R>;
  send(request: R): Promise<unknown>;
  /** Whether the thing still waits: true, false (answered, closed or gone), or null when Fractal cannot say. */
  stillWaiting(): Promise<boolean | null>;
  wait(ms: number): Promise<void>;
};

export type AnswerOutcome =
  | { ok: true; how: 'sent' | 'already' }
  | { ok: false; how: 'refused'; error: unknown }
  | { ok: false; how: 'unconfirmed'; error: unknown; check(): Promise<AnswerOutcome> };

/** How many requests a stale fence may cost before the refusal is shown. */
export const ANSWER_TRIES = 8;
/** How long to wait before sending the identical request again after a lost reply. */
export const REPEAT_AFTER_MS = 1000;

export async function submitAnswer<R>(ports: AnswerPorts<R>): Promise<AnswerOutcome> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < ANSWER_TRIES; attempt++) {
    let request: R;
    let ambiguous = false;
    try { request = await ports.build(); } catch (error) { return { ok: false, how: 'refused', error }; }
    try { requireCompletedActionReceipt(request, await ports.send(request)); return { ok: true, how: 'sent' }; } catch (error) { last = error; ambiguous = !answerNotApplied(error); }
    if (outcomeUnknown(last)) {
      // Fractal may have taken it: the identical request gets Fractal's first answer.
      await ports.wait(REPEAT_AFTER_MS);
      try { requireCompletedActionReceipt(request, await ports.send(request)); return { ok: true, how: 'sent' }; } catch (error) { last = error; }
    }
    // Whatever went wrong, an answer that is no longer waited for is an answer that landed.
    if ((await settled(ports)) === true) return { ok: true, how: 'already' };
    if (ambiguous) return unresolved(ports, request, last, true);
    // Only a refusal that guarantees nothing was applied is tried again with a new request.
    if (!answerNotApplied(last) || attempt + 1 >= ANSWER_TRIES) return unresolved(ports, request, last);
    await ports.wait(wait);
    wait = Math.min(wait * 2, 2000);
  }
  return { ok: false, how: 'refused', error: last };
}

/** True when the thing no longer waits; false while it does or when Fractal cannot say. */
async function settled<R>(ports: AnswerPorts<R>): Promise<boolean> {
  try { return (await ports.stillWaiting()) === false; } catch { return false; }
}

/** Unknown delivery keeps the original immutable identity and transport. Only explicit Check retries it. */
function unresolved<R>(ports: AnswerPorts<R>, request: R, error: unknown, ambiguous = false): AnswerOutcome {
  if (!ambiguous && answerNotApplied(error)) return { ok: false, how: 'refused', error };
  return { ok: false, how: 'unconfirmed', error, check: async () => {
    try { requireCompletedActionReceipt(request, await ports.send(request)); return { ok: true, how: 'sent' }; } catch (error) {
      if (await settled(ports)) return { ok: true, how: 'already' };
      return unresolved(ports, request, error, true);
    }
  } };
}

/** A resolved HTTP request is not an action receipt. Accepted, missing or unrelated receipts stay uncertain. */
export function requireCompletedActionReceipt(request: unknown, receipt: unknown): void {
  const id = request && typeof request === 'object' && 'id' in request ? request.id : undefined;
  const value = receipt && typeof receipt === 'object' && 'value' in receipt ? receipt.value : undefined;
  if (typeof id === 'string' && value && typeof value === 'object' && 'kind' in value && value.kind === 'action-result' && 'action_id' in value && value.action_id === id && 'status' in value && value.status === 'completed') return;
  throw new Error('The original action has no matching completed receipt. Check delivery before acting again.');
}

/** Unknown/replay-unsafe metadata overrides the older stale/rate-limited shorthand. */
export function answerNotApplied(error: unknown): boolean {
  if (appliedAnswer(error) === 'unknown') return false;
  if (error instanceof ClientError) {
    const details = error.response.details as Record<string, unknown> | undefined;
    if (details?.replay_unsafe !== undefined && details.replay_unsafe !== false) return false;
  }
  return notApplied(error);
}
