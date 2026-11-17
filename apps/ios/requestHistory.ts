import type { FractalClient, RequestHistoryItem, RequestHistoryPage, RequestHistoryRelation, SourceEpisode } from '../../clients/typescript/fractal-client';

export const REQUEST_HISTORY_LIMIT = 20;
export const requestHistoryKey = (key: SourceEpisode): string => JSON.stringify([key.source_id, key.episode]);

/** Presentation only: the server owns membership, episode state and authorization. */
export const requestHistoryRows = (page: RequestHistoryPage): RequestHistoryItem[] => {
  const rows = new Map<string, RequestHistoryItem>();
  for (const item of [...page.boundary_items, ...page.items]) rows.set(requestHistoryKey(item.key), item);
  return [...rows.values()];
};

export const requestHistoryRelations = (page: RequestHistoryPage): RequestHistoryRelation[] => {
  const visible = new Set(requestHistoryRows(page).map(item => requestHistoryKey(item.key)));
  return page.relations.filter(relation => visible.has(requestHistoryKey(relation.predecessor)) && visible.has(requestHistoryKey(relation.request)));
};

export type RequestHistorySnapshot =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; page: RequestHistoryPage; loadingMore: boolean; issue?: string };

type HistoryClient = Pick<FractalClient, 'threadRequestHistory'>;

/** One authorized page at a time. A continuation reauthorizes rows; old pages must
 * not be unioned into it, since permissions and standing answers can change. */
export class RequestHistoryReader {
  private snapshot: RequestHistorySnapshot = { kind: 'loading' };
  private listeners = new Set<() => void>();
  private active?: AbortController;
  private generation = 0;
  private previousCursor?: string;

  constructor(private readonly client: HistoryClient, private readonly threadId: string) {}

  getSnapshot = (): RequestHistorySnapshot => this.snapshot;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  private publish = (snapshot: RequestHistorySnapshot): void => {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener();
  };
  load = (): Promise<void> => {
    this.previousCursor = undefined;
    return this.read();
  };
  more = (): Promise<void> => {
    const current = this.snapshot;
    if (current.kind !== 'ready' || current.loadingMore || !current.page.next_cursor) return Promise.resolve();
    const cursor = current.page.next_cursor;
    if (this.previousCursor === cursor) {
      this.publish({ ...current, issue: 'History continuation is unavailable. Refresh history.' });
      return Promise.resolve();
    }
    this.previousCursor = cursor;
    return this.read(cursor);
  };
  close = (): void => {
    this.generation++;
    this.active?.abort();
    this.active = undefined;
    // No authorized content survives a released reader (navigation/session change).
    this.publish({ kind: 'loading' });
    this.previousCursor = undefined;
  };
  private read = async (cursor?: string): Promise<void> => {
    this.active?.abort();
    const controller = new AbortController();
    this.active = controller;
    const generation = ++this.generation;
    // Drop the prior page before every read, including failures and revocations.
    this.publish({ kind: 'loading' });
    try {
      const { value } = await this.client.threadRequestHistory(this.threadId, { limit: REQUEST_HISTORY_LIMIT, ...(cursor ? { cursor } : {}) }, { signal: controller.signal });
      if (controller.signal.aborted || generation !== this.generation) return;
      this.publish({ kind: 'ready', page: value, loadingMore: false });
    } catch {
      if (controller.signal.aborted || generation !== this.generation) return;
      this.publish({ kind: 'error', message: 'Request history is unavailable. Refresh to try again.' });
    }
  };
}
