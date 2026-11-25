// Answers with unknown delivery are never replayed. A read-only prompt check can
// establish that no answer is needed, not that this particular answer landed.
import { notApplied } from '../../clients/typescript/fractal-client';

export type AnswerPorts<R> = {
  /** A fresh request with its own key, fenced to the viewed revision; a known-refused retry reads a fresh snapshot. */
  build(): Promise<R>;
  send(request: R): Promise<unknown>;
  /** Whether the thing still waits: true, false (answered, closed or gone), or null when Fractal cannot say. */
  stillWaiting(): Promise<boolean | null>;
  wait(ms: number): Promise<void>;
};

export type AnswerOutcome =
  | { ok: true; how: 'sent' | 'already' }
  | { ok: false; error: unknown };

/** How many explicit nonapplication refusals to allow before showing the last refusal. */
export const ANSWER_TRIES = 8;

export async function submitAnswer<R>(ports: AnswerPorts<R>): Promise<AnswerOutcome> {
  let last: unknown;
  let wait = 50;
  for (let attempt = 0; attempt < ANSWER_TRIES; attempt++) {
    let request: R;
    try { request = await ports.build(); } catch (error) { return { ok: false, error }; }
    try { await ports.send(request); return { ok: true, how: 'sent' }; } catch (error) { last = error; }
    // A satisfied prompt needs no further answer, regardless of which answer satisfied it.
    if ((await settled(ports)) === true) return { ok: true, how: 'already' };
    // Only a refusal that guarantees nothing was applied is tried again with a new request.
    if (!notApplied(last) || attempt + 1 >= ANSWER_TRIES) return { ok: false, error: last };
    await ports.wait(wait);
    wait = Math.min(wait * 2, 2000);
  }
  return { ok: false, error: last };
}

/** True when the thing no longer waits; false while it does or when Fractal cannot say. */
async function settled<R>(ports: AnswerPorts<R>): Promise<boolean> {
  try { return (await ports.stillWaiting()) === false; } catch { return false; }
}
