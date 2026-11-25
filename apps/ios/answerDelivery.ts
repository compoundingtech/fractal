// Answers with unknown delivery are never replayed. Read-only checks may establish that
// the prompt no longer waits, but cannot establish which answer satisfied it.
import { appliedAnswer, ClientError, notApplied } from '../../clients/typescript/fractal-client';

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

/** How many explicit nonapplication refusals to allow before showing the last refusal. */
export const ANSWER_TRIES = 8;

export async function submitAnswer<R>(ports: AnswerPorts<R>): Promise<AnswerOutcome> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < ANSWER_TRIES; attempt++) {
    let request: R;
    let ambiguous = false;
    try { request = await ports.build(); } catch (error) { return { ok: false, how: 'refused', error }; }
    try { requireCompletedActionReceipt(request, await ports.send(request)); return { ok: true, how: 'sent' }; } catch (error) { last = error; ambiguous = !answerNotApplied(error); }
    // This does not prove our answer landed; it only means no further answer is needed.
    if ((await settled(ports)) === true) return { ok: true, how: 'already' };
    if (ambiguous || attempt + 1 >= ANSWER_TRIES) return unresolved(ports, last);
    await ports.wait(wait);
    wait = Math.min(wait * 2, 2000);
  }
  return { ok: false, how: 'refused', error: last };
}

/** True when the thing no longer waits; false while it does or when Fractal cannot say. */
async function settled<R>(ports: AnswerPorts<R>): Promise<boolean> {
  try { return (await ports.stillWaiting()) === false; } catch { return false; }
}

/** Check reads prompt state only; it never posts, rebuilds, signs or refreshes the unknown action. */
function unresolved<R>(ports: AnswerPorts<R>, error: unknown): AnswerOutcome {
  if (answerNotApplied(error)) return { ok: false, how: 'refused', error };
  return { ok: false, how: 'unconfirmed', error, check: async () => {
    if (await settled(ports)) return { ok: true, how: 'already' };
    return unresolved(ports, error);
  } };
}

/** A resolved HTTP request is not an action receipt. Accepted, missing or unrelated receipts stay uncertain. */
export function requireCompletedActionReceipt(request: unknown, receipt: unknown): void {
  const id = request && typeof request === 'object' && 'id' in request ? request.id : undefined;
  const value = receipt && typeof receipt === 'object' && 'value' in receipt ? receipt.value : undefined;
  if (typeof id === 'string' && value && typeof value === 'object' && 'kind' in value && value.kind === 'action-result' && 'action_id' in value && value.action_id === id && 'status' in value && value.status === 'completed') return;
  throw new Error('The original action has no matching completed receipt. Check delivery before acting again.');
}

/** Explicit proof is required; defensive replay-unsafe metadata can only remove permission to retry. */
export function answerNotApplied(error: unknown): boolean {
  if (appliedAnswer(error) === 'unknown') return false;
  if (error instanceof ClientError) {
    const details = error.response.details as Record<string, unknown> | undefined;
    if (details?.replay_unsafe !== undefined && details.replay_unsafe !== false) return false;
  }
  return notApplied(error);
}
