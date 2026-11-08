import type { TimelineEntry } from '../../clients/typescript/fractal-client';
import type { QualityStorage } from './quality';
import { rememberBounded } from './boundedCache';

// A conversation's read position, kept on this device for the paired gateway and person: the time
// of the newest prose the person has seen there. Same contract as the web app
// (apps/fractal-web/src/conversation/unread.ts, src/web/readPosition.ts).

export const READ_POSITIONS_KEY = 'st3.read-positions.v1';
/** The roster stamps the same words a moment after the entry; prose this close is what was seen. */
export const ROSTER_SKEW_MS = 2_000;
export const UNREAD_DOT = '•';
export const UNREAD_LABEL = 'New messages';
const MAX_CONVERSATIONS = 512, MAX_OWNERS = 8;

const ms = (value: string | null | undefined): number | undefined => {
  if (!value) return undefined;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : undefined;
};
const personal = /^person\//;

/** The other side's prose: assistant text and messages no person wrote; never tools, status or prompts. */
export function countsAsNew(entry: Pick<TimelineEntry, 'type' | 'role' | 'body'>): boolean {
  if (entry.type === 'content') return entry.role === 'assistant';
  if (entry.type === 'message') {
    const body = entry.body as { author?: unknown; from?: unknown };
    return entry.role !== 'user' && !personal.test(String(body.author ?? '')) && !personal.test(String(body.from ?? ''));
  }
  return false;
}

/** Where an open conversation's position goes: the newest of the roster's prose and the loaded prose. */
export function seenTail(lastProseAt: string | null | undefined, entries: readonly Pick<TimelineEntry, 'type' | 'role' | 'body' | 'timestamp'>[]): number | undefined {
  let latest = ms(lastProseAt);
  for (const entry of entries) {
    if (!countsAsNew(entry)) continue;
    const at = ms(entry.timestamp);
    if (at !== undefined && (latest === undefined || at > latest)) latest = at;
  }
  return latest;
}

/** A row's or tab's mark: a dot when the roster knows prose newer than the position. Unknown position or no roster prose: none. */
export function unreadMark(seenAtMs: number | undefined, lastProseAt: string | null | undefined): typeof UNREAD_DOT | undefined {
  const prose = ms(lastProseAt);
  return seenAtMs !== undefined && prose !== undefined && prose > seenAtMs + ROSTER_SKEW_MS ? UNREAD_DOT : undefined;
}

type Stored = Record<string, Array<[string, number]>>;

/** Positions per gateway and person, forward-only, saved best-effort in one bounded local key. */
export class ReadPositions {
  private owner = '';
  private all: Map<string, Map<string, number>> = new Map();
  private loaded: Promise<void> | null = null;
  private writing: Promise<void> | null = null;
  private dirty = false;
  private readonly listeners = new Set<() => void>();
  /** Changes whenever a position or the owner does: a snapshot for `useSyncExternalStore`. */
  revision = 0;
  constructor(private readonly storage: QualityStorage) {}

  /** Selects the gateway and person whose positions are read and advanced. */
  async load(gateway: string, person: string): Promise<void> {
    this.loaded ??= this.restore();
    await this.loaded;
    const owner = gateway && person ? `${gateway}|${person}` : '';
    if (owner === this.owner) return;
    this.owner = owner;
    this.emit();
  }
  read(conversation: string): number | undefined {
    return this.owner ? this.all.get(this.owner)?.get(conversation) : undefined;
  }
  advance(conversation: string, atMs: number): void {
    if (!this.owner || !Number.isFinite(atMs)) return;
    const current = this.read(conversation);
    if (current !== undefined && current >= atMs) return;
    const positions = this.all.get(this.owner) ?? new Map<string, number>();
    rememberBounded(positions, conversation, atMs, MAX_CONVERSATIONS);
    rememberBounded(this.all, this.owner, positions, MAX_OWNERS);
    this.dirty = true;
    void this.drain();
    this.emit();
  }
  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  /** Resolves once every advance so far is written. */
  flush(): Promise<void> { return this.writing ?? Promise.resolve(); }

  private emit() { this.revision++; for (const listener of this.listeners) listener(); }
  private async restore(): Promise<void> {
    let raw: string | null;
    try { raw = await this.storage.getItem(READ_POSITIONS_KEY); } catch { return; }
    if (!raw) return;
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return;
      for (const [owner, entries] of Object.entries(parsed as Stored).slice(-MAX_OWNERS)) {
        if (!Array.isArray(entries)) continue;
        const positions = this.all.get(owner) ?? new Map<string, number>();
        for (const entry of entries.slice(-MAX_CONVERSATIONS)) {
          if (!Array.isArray(entry) || typeof entry[0] !== 'string' || typeof entry[1] !== 'number' || !Number.isFinite(entry[1])) continue;
          // An advance made while reading wins when it is newer.
          if ((positions.get(entry[0]) ?? -Infinity) < entry[1]) positions.set(entry[0], entry[1]);
        }
        this.all.set(owner, positions);
      }
    } catch { /* a damaged key starts over: nothing is new */ }
  }
  private drain(): Promise<void> {
    this.writing ??= (async () => {
      await this.loaded;
      while (this.dirty) {
        this.dirty = false;
        const stored: Stored = Object.fromEntries([...this.all].map(([owner, positions]) => [owner, [...positions]]));
        try { await this.storage.setItem(READ_POSITIONS_KEY, JSON.stringify(stored)); } catch { /* best-effort */ }
      }
      this.writing = null;
    })();
    return this.writing;
  }
}
