import { applyWindow, plainMessage, type CollectionFrame, type CollectionStream, type CollectionWindow, type Snapshot } from '../../clients/typescript/st3-client';
import { inventoryRows, type InventoryLists, type InventoryName } from '@smalltalk/st3-views';

export type InventoryHandlers<K extends InventoryName> = {
  onRows: (rows: InventoryLists[K], hasMore: boolean, snapshot: Snapshot) => void;
  onIssue: (issue: string) => void;
};
type Listener = InventoryHandlers<InventoryName>;
type Held = { name: InventoryName; hours?: number; listeners: Set<Listener>; window?: CollectionWindow; leave?: ReturnType<typeof setTimeout>; issue: string };

/** Optional screen windows share the feed socket and the missions' leave grace. Three usage
 * periods plus the other five inventories and existing seven slots fit the sixteen-slot bound. */
export class InventoryFeed {
  private held = new Map<string, Held>();
  private stream: CollectionStream | undefined;
  private granted = false;
  constructor(private readonly graceMs = 30_000) {}

  setGranted(granted: boolean): void {
    if (granted === this.granted) return;
    this.granted = granted;
    for (const [id, follow] of this.held) {
      follow.window = undefined;
      if (granted) this.subscribe(id, follow); else this.stream?.unsubscribe(id);
    }
  }

  open(stream: CollectionStream): void {
    this.stream = stream;
    for (const [id, follow] of this.held) { follow.window = undefined; this.subscribe(id, follow); }
  }
  suspend(): void { this.stream = undefined; for (const follow of this.held.values()) follow.window = undefined; }
  close(): void { this.suspend(); for (const follow of this.held.values()) clearTimeout(follow.leave); this.held.clear(); }

  watch<K extends InventoryName>(name: K, handlers: InventoryHandlers<K>, hours?: 24 | 168 | 720): () => void {
    const id = name === 'usage' ? `usage-${hours ?? 24}` : name;
    let follow = this.held.get(id);
    if (!follow) {
      follow = { name, hours, listeners: new Set(), issue: '' };
      this.held.set(id, follow);
      this.subscribe(id, follow);
    }
    clearTimeout(follow.leave); follow.leave = undefined;
    const listener = { ...handlers } as Listener;
    follow.listeners.add(listener);
    if (follow.window) listener.onRows(inventoryRows(name, follow.window), follow.window.hasMore, follow.window.snapshot);
    listener.onIssue(follow.issue);
    const held = follow;
    let done = false;
    return () => {
      if (done) return;
      done = true;
      held.listeners.delete(listener);
      if (held.listeners.size) return;
      held.leave = setTimeout(() => {
        if (this.held.get(id) !== held || held.listeners.size) return;
        this.stream?.unsubscribe(id); this.held.delete(id);
      }, this.graceMs);
    };
  }

  refresh(names?: readonly InventoryName[]): void {
    for (const [id, follow] of this.held) if ((!names || names.includes(follow.name)) && follow.listeners.size) this.subscribe(id, follow);
  }

  frame(frame: CollectionFrame): boolean {
    if (!('id' in frame) || typeof frame.id !== 'string') return false;
    const follow = this.held.get(frame.id);
    if (!follow || !this.granted) return false;
    if ((frame.kind === 'snapshot' || frame.kind === 'changes') && frame.collection !== 'ordered-memberships') {
      const next = applyWindow(follow.window, frame);
      if (!next) { this.subscribe(frame.id, follow); return true; }
      follow.window = next; follow.issue = '';
      for (const listener of follow.listeners) {
        listener.onIssue(''); listener.onRows(inventoryRows(follow.name, next), next.hasMore, next.snapshot);
      }
    } else if (frame.kind === 'resync' || frame.kind === 'error') {
      // The daemon retains retryable resync subscriptions. Permanent refusals stay visible;
      // no HTTP fallback or local retry conceals an admitted subscription failure.
      follow.issue = `${plainMessage(frame.code, frame.message ?? 'The inventory stream needs a fresh snapshot.')} ${frame.kind === 'resync' && frame.retryable ? '· trying again' : ''}`.trim();
      for (const listener of follow.listeners) listener.onIssue(follow.issue);
    }
    return true;
  }

  private subscribe(id: string, follow: Held): void {
    if (!this.granted || !this.stream) return;
    follow.window = undefined;
    const filters = follow.name === 'usage' ? { since_ms: Date.now() - (follow.hours ?? 24) * 3_600_000 } : {};
    this.stream.subscribe(id, follow.name, 200, filters);
  }
}
