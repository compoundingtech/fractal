import type { ClientVisibleState } from '../../clients/typescript/fractal-client/ClientControl.generated';
import { report, spaced } from '@fractal/views';
import { markdown } from './markdown';

/** Only report exact text when the normal question renderer preserves it. */
export const observedQuestion = (text: string): string | undefined => {
  if (report(text)) return;
  const blocks = markdown(spaced(text));
  if (blocks.length !== 1 || blocks[0].kind !== 'text') return;
  const runs = blocks[0].runs;
  return runs.every(run => run.style === 'plain') && runs.map(run => run.text).join('') === text ? text : undefined;
};

// Published only by committed normal renderers, never by command dispatch.
const snapshots = new Map<string, ClientVisibleState>();
const listeners = new Set<() => void>();
const notify = () => { for (const listener of listeners) listener(); };
export const committedVisible = (routeKey: string): ClientVisibleState | undefined => snapshots.get(routeKey);
export const publishVisible = (routeKey: string, state: ClientVisibleState | undefined): void => {
  if (state) snapshots.set(routeKey, state);
  else snapshots.delete(routeKey);
  notify();
};
export const subscribeVisible = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
