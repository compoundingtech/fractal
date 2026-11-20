import type { ThreadStartOrigin } from './store';

export type SavedThreadStart = { target: string; origin: ThreadStartOrigin; title: string };
export const startRecoveryKey = (target: string): string => `thread-start-recovery/${target}`;

/** Presentation only: never persist a bound action, operation key or signature. */
export const savedThreadStarts = (cache: ReadonlyMap<string, string>): readonly SavedThreadStart[] => {
  const starts: SavedThreadStart[] = [];
  for (const [key, raw] of cache) {
    if (!key.startsWith('thread-start-recovery/')) continue;
    try {
      const value = JSON.parse(raw);
      if (typeof value?.target !== 'string' || key !== startRecoveryKey(value.target)
        || typeof value.title !== 'string' || typeof value.origin?.sourceId !== 'string'
        || typeof value.origin.episode !== 'string' || typeof value.origin.agentId !== 'string'
        || !cache.has(`thread-draft/start/${value.target}`)) continue;
      starts.push({ target: value.target, title: value.title, origin: {
        sourceId: value.origin.sourceId, episode: value.origin.episode, agentId: value.origin.agentId,
      } });
    } catch { /* A malformed discovery record cannot authorize or fabricate a Start. */ }
  }
  return starts;
};
