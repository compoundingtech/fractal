// Asks to the person that Fractal resolved by their deadline, as Now lists them under Auto-resolved:
// what was asked, who asked, the default that applied and when. While the override window is open
// (Fractal says so with `action_parameters`), the person may still give their own answer: one of the
// ask's other named answers replaces the default, and the asker is told.
import { ago, type KeptAttention } from '@fractal/views';
import { ClientError, notApplied, plainError, type AutoResolvedAsk, type RequestAnswerOption } from '../../clients/typescript/fractal-client';

export type OverrideChoice = Pick<RequestAnswerOption, 'id' | 'label' | 'outcome'>;
/** What `work.override` sends as the answer: the named answer's ID, with the person's words when it asks for changes. */
export type OverrideAnswer = { id: string; text?: string };
export type AutoResolvedRow = {
  key: string;
  ask: AutoResolvedAsk;
  title: string;
  asker: string;
  context: string;
  /** The named answers that may replace the default; empty when the override is not offered. */
  choices: OverrideChoice[];
};

export const AUTO_RESOLVED_TITLE = 'Auto-resolved';
export const OVERRIDE_TITLE = 'Use my answer instead';
/** Fractal stopped its scan at the cap: asks may be missing, also when none are listed. */
export const INCOMPLETE_AUTO_RESOLVED = 'Fractal could not check every recent ask, so some auto-resolved asks may be missing here.';

/** The words every client shows before an override, verbatim. */
export function overrideCopy(asker: string): string {
  return `Your answer replaces the default. Work already done on the default is not undone; ${asker} is told.`;
}

/** The answers that may replace the default: the ask's named answers other than the default, only while Fractal offers the override. */
export function overrideChoices(ask: AutoResolvedAsk): OverrideChoice[] {
  if (!ask.action_parameters?.['work.override']) return [];
  return (ask.request?.answers ?? []).filter(answer => answer.id !== ask.default_answer?.id).map(({ id, label, outcome }) => ({ id, label, ...(outcome ? { outcome } : {}) }));
}

/** Whether the chosen answer needs the person's words: requesting changes says what should change. */
export function needsWords(choice: OverrideChoice): boolean {
  return choice.outcome === 'request_changes';
}

export const WORDS_REQUIRED = 'Say what should change; an empty answer is not sent.';

/** The answer to send for a choice: its ID, plus the trimmed words when it requests changes. Empty words are refused with `WORDS_REQUIRED`. */
export function overrideAnswer(choice: OverrideChoice, words?: string): OverrideAnswer | string {
  if (!needsWords(choice)) return { id: choice.id };
  const text = words?.trim() ?? '';
  return text ? { id: choice.id, text } : WORDS_REQUIRED;
}

/** The section shows while there are rows, a read failed, or Fractal's list is incomplete (even with no rows yet). */
export function autoResolvedVisible(rows: readonly AutoResolvedRow[], read: { issue: string; complete: boolean }): boolean {
  return rows.length > 0 || !!read.issue || !read.complete;
}

/**
 * One override in flight per ask: a second send for the same episode is refused until the first settles,
 * accepted or refused. `run` resolves to whether the override was sent; a refused start resolves false.
 */
export async function singleOverride(pending: Set<string>, episode: string, run: () => Promise<boolean>, changed: () => void = () => {}): Promise<boolean> {
  if (pending.has(episode)) return false;
  pending.add(episode); changed();
  try { return await run(); } finally { pending.delete(episode); changed(); }
}

/** One row per ask, newest resolution first as Fractal lists them; `asker` names the requester, falling back to the ID. */
export function autoResolvedRows(asks: readonly AutoResolvedAsk[], asker: (id: string) => string, now = Date.now()): AutoResolvedRow[] {
  return asks.map(ask => {
    const name = asker(ask.requester_id);
    return {
      key: ask.resolution_id,
      ask,
      title: ask.title,
      asker: name,
      context: [ask.default_answer ? `Default: ${ask.default_answer.label}` : 'No default', `From: ${name}`, `Resolved ${ago(ask.resolved_at, now)} ago`].join(' · '),
      choices: overrideChoices(ask),
    };
  });
}

/** The auto-resolved ask a recently closed Now item stands for, by its episode, when it can still be overridden. */
export function closedOverride(item: KeptAttention, rows: readonly AutoResolvedRow[]): AutoResolvedRow | undefined {
  if (!item.closedElsewhere || !item.episode) return undefined;
  return rows.find(row => row.ask.episode === item.episode && row.choices.length > 0);
}

/** The idempotency key: one override per episode and answer, the same on every try. */
export function overrideKey(episode: string, answerId: string): string {
  return `person-override:${episode}:${answerId}`;
}

/** `work.override`: the action ID and key are stable across retries, so a repeat returns Fractal's first answer. */
export function overrideRequest(ask: AutoResolvedAsk, answer: OverrideAnswer, fence: { snapshot_id: string; subject_revisions: Record<string, string> }) {
  const parameters = ask.action_parameters?.['work.override'];
  if (!parameters) throw new Error('This ask can no longer be overridden.');
  const key = overrideKey(parameters.episode, answer.id);
  return { id: `action/${key}`, idempotency_key: key, fence, parameters: { target_id: parameters.target_id, episode: parameters.episode, answer: answer.text ? { id: answer.id, text: answer.text } : { id: answer.id } } };
}

/**
 * Whether explicit proof of nonapplication permits another dispatch, except a stale fence
 * (the override window closed or the ask was answered). Unknown outcomes never authorize replay.
 */
export function overrideRetryable(error: unknown): boolean {
  if (error instanceof ClientError && error.response.code === 'stale-fence') return false;
  return notApplied(error);
}

/** A refused override as the person reads it. */
export function overrideRefusal(error: unknown): string {
  if (error instanceof ClientError && error.response.code === 'stale-fence') return 'This ask can no longer take your answer: its override window closed, or it was already answered.';
  if (error instanceof ClientError && error.response.code === 'forbidden') return 'Only the person who was asked can replace its default.';
  return plainError(error);
}
