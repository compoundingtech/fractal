import type { ThreadStartOrigin } from './store';
import type { LegacyStartRecovery } from './outboxMessage.ts';

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

/** Commit packetless metadata before removing the exact old draft/discovery pair. */
export const importSavedThreadStarts = async (
  cache: Map<string, string>, current: () => boolean,
  commit: (entry: LegacyStartRecovery) => Promise<void>, save: () => Promise<void>,
): Promise<void> => {
  for (const start of savedThreadStarts(cache)) {
    const key = `thread-draft/start/${start.target}`, discoveryKey = startRecoveryKey(start.target);
    const serializedDraft = cache.get(key), discovery = cache.get(discoveryKey);
    if (serializedDraft === undefined || discovery === undefined || !current()) return;
    let text: unknown;
    try { text = JSON.parse(serializedDraft)?.text; } catch { continue; }
    if (typeof text !== 'string') continue;
    await commit({ id: `legacy-start/${start.target}`, message: { kind: 'start', target: start.target, title: start.title, origin: start.origin }, text, serializedDraft });
    if (!current() || cache.get(key) !== serializedDraft || cache.get(discoveryKey) !== discovery) continue;
    cache.delete(key); cache.delete(discoveryKey);
    try { await save(); }
    catch (error) {
      // The durable import is idempotent; retain the old source if draft-store removal failed.
      if (current()) { if (!cache.has(key)) cache.set(key, serializedDraft); if (!cache.has(discoveryKey)) cache.set(discoveryKey, discovery); }
      throw error;
    }
  }
};
