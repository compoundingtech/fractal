import type { TimelineEntry } from '../../clients/typescript/fractal-client';
import { ClientError } from '../../clients/typescript/fractal-client/Client.generated.ts';
import type { QualityStorage } from './quality';
import { rememberBounded } from './boundedCache';

// A conversation's read position for the paired gateway and person: the time of the newest prose the
// person has seen there. Same contract as the web app and the Fractal terminal UI. A daemon that grants
// `read-positions` holds the person's positions for every device: they are listed on connect and on
// each frame of the `read-positions` collection, and merged here by max; this device's advances are
// sent as `read-position.advance`. This device keeps the merged positions and the advances not sent
// yet only as an offline cache; without the capability (an older daemon) positions stay here alone.

export const READ_POSITIONS_KEY = 'st3.read-positions.v1';
/** Conversations whose position moved here and that the daemon has not confirmed yet, per owner. */
export const UNSENT_READ_POSITIONS_KEY = 'st3.read-positions.unsent.v1';
export const READ_POSITIONS_CAPABILITY = 'read-positions';
/** The roster stamps the same words a moment after the entry; prose this close is what was seen. */
export const ROSTER_SKEW_MS = 2_000;
/** At most one advance per conversation is sent this often; reading moves positions many times a second. */
export const ADVANCE_EVERY_MS = 2_000;
/** Capped backoff while a peer is unreachable: unsent advances and failed position lists. */
export const ADVANCE_RETRY_MS = [1_000, 2_000, 5_000, 10_000, 30_000];
export const UNREAD_DOT = '•';
export const UNREAD_LABEL = 'New messages';
const MAX_CONVERSATIONS = 512, MAX_OWNERS = 8, MAX_PAGES = 20;

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

/** The newest presented prose; roster timestamps never establish what the person saw. */
export function seenTail(entries: readonly Pick<TimelineEntry, 'id' | 'type' | 'role' | 'body' | 'timestamp'>[], presented: ReadonlySet<string>): number | undefined {
  let latest: number | undefined;
  for (const entry of entries) {
    if (!countsAsNew(entry)) continue;
    const body = entry.body as { message_id?: string };
    if (![...presented].some(id => id === entry.id || id.startsWith(`${entry.id}#`) || (entry.type === 'message' && id === body.message_id))) continue;
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

/** Whether the daemon keeps read positions for every device; without it they stay on this one. */
export function syncsReadPositions(capabilities: readonly { id: string; version: number; state: string }[] | undefined): boolean {
  return capabilities?.some(capability => capability.id === READ_POSITIONS_CAPABILITY && capability.version >= 1 && capability.state === 'granted') ?? false;
}

/** One of the person's positions as the daemon lists it. */
export type ListedReadPosition = { conversation: string; at: string };
/** The daemon side of synced positions, for one person. */
export type ReadPositionRemote = {
  /** Every position the daemon holds for the person. */
  list(): Promise<readonly ListedReadPosition[]>;
  /** Sends one advance; resolves with the value the daemon holds after the merge, when it says. */
  advance(conversation: string, at: string): Promise<string | undefined>;
};
export type ReadPositionAdvanceRequest = { owner: string; conversation: string; at: string };
/** The client calls behind a remote: one `read-positions.list` page and one `read-position.advance` action. */
export type ReadPositionPorts = {
  list(cursor: string | undefined): Promise<{ items: readonly unknown[]; page: { has_more: boolean; next_cursor?: string | null } }>;
  advance(parameters: ReadPositionAdvanceRequest): Promise<{ read_position_at?: string }>;
};

const isListed = (item: unknown, person: string): item is ListedReadPosition & { kind: 'read-position'; owner: string } => {
  if (typeof item !== 'object' || item === null) return false;
  const row = item as Record<string, unknown>;
  return row.kind === 'read-position' && row.owner === person && typeof row.conversation === 'string' && typeof row.at === 'string';
};

/** The person's positions over every page of `read-positions.list`, and advances owned by the person. */
export function readPositionRemote(person: string, ports: ReadPositionPorts): ReadPositionRemote {
  return {
    async list() {
      const listed: ListedReadPosition[] = [];
      const seen = new Set<string>();
      let cursor: string | undefined;
      for (let number = 0; number < MAX_PAGES; number++) {
        const page = await ports.list(cursor);
        for (const item of page.items) if (isListed(item, person)) listed.push({ conversation: item.conversation, at: item.at });
        const next = page.page.next_cursor ?? undefined;
        // A cursor that does not move ends the read: what was listed still merges.
        if (!page.page.has_more || !next || seen.has(next)) break;
        seen.add(next);
        cursor = next;
      }
      return listed;
    },
    async advance(conversation, at) {
      return (await ports.advance({ owner: person, conversation, at })).read_position_at;
    },
  };
}

export type ReadPositionOptions = { everyMs?: number; retryDelaysMs?: readonly number[] };
type Stored = Record<string, Array<[string, number]>>;
type Send = { inFlight: boolean; lastSentAt: number; failures: number; timer?: ReturnType<typeof setTimeout> };
/** A connection to the daemon for one owner: what it confirmed, and each conversation's send. */
type Sync = {
  owner: string; remote: ReadPositionRemote; confirmed: Map<string, number>; sends: Map<string, Send>;
  listing: Promise<void> | null; again: boolean; listIssue?: string; listFailures: number;
  listTimer?: ReturnType<typeof setTimeout>;
};
const ownerKey = (gateway: string, person: string) => gateway && person ? `${gateway}|${person}` : '';

/** Only an unreachable peer/transport warrants a list retry, never a local defect or a refusal. */
const listUnreachable = (error: unknown): boolean => {
  if (error instanceof ClientError) {
    return error.response.code === 'remote-unavailable'
      || error.response.code === 'http-error' && [502, 503, 504].includes(error.status);
  }
  return error instanceof Error && /network request failed|failed to fetch|networkerror|econnrefused|econnreset|enotfound|etimedout/i.test(error.message);
};

/** Positions per gateway and person, forward-only, merged with the daemon's when connected, cached in bounded local keys. */
export class ReadPositions {
  private owner = '';
  private all: Map<string, Map<string, number>> = new Map();
  private unsent: Map<string, Set<string>> = new Map();
  private sync: Sync | undefined;
  private loaded: Promise<void> | null = null;
  private writing: Promise<void> | null = null;
  private dirty = false;
  private readonly listeners = new Set<() => void>();
  private readonly everyMs: number;
  private readonly retryDelaysMs: readonly number[];
  /** Changes whenever a position or the owner does: a snapshot for `useSyncExternalStore`. */
  revision = 0;
  constructor(private readonly storage: QualityStorage, options: ReadPositionOptions = {}) {
    this.everyMs = options.everyMs ?? ADVANCE_EVERY_MS;
    this.retryDelaysMs = options.retryDelaysMs?.length ? options.retryDelaysMs : ADVANCE_RETRY_MS;
  }

  /** Selects the gateway and person whose positions are read and advanced. */
  async load(gateway: string, person: string): Promise<void> {
    await this.restored();
    const owner = ownerKey(gateway, person);
    if (owner === this.owner) return;
    this.owner = owner;
    this.emit();
  }
  read(conversation: string): number | undefined {
    return this.owner ? this.all.get(this.owner)?.get(conversation) : undefined;
  }
  /** The selected owner's last list failure; remains visible until success or disconnect. */
  get listIssue(): string | undefined {
    return this.sync?.owner === this.owner ? this.sync.listIssue : undefined;
  }
  advance(conversation: string, atMs: number): void {
    if (!this.owner || !Number.isFinite(atMs) || !this.raise(this.owner, conversation, atMs)) return;
    const unsent = this.unsent.get(this.owner) ?? new Set<string>();
    unsent.delete(conversation);
    unsent.add(conversation);
    for (const oldest of unsent) { if (unsent.size <= MAX_CONVERSATIONS) break; unsent.delete(oldest); }
    this.unsent.set(this.owner, unsent);
    this.save();
    this.emit();
    if (this.sync?.owner === this.owner) this.schedule(this.sync, conversation);
  }

  /**
   * Syncs one gateway's person with its daemon: lists the positions now and on each `relist`, and
   * sends this device's advances, the unsent ones first. A second call replaces the first.
   */
  connect(gateway: string, person: string, remote: ReadPositionRemote): () => void {
    this.disconnect();
    const owner = ownerKey(gateway, person);
    if (!owner) return () => {};
    const sync: Sync = { owner, remote, confirmed: new Map(), sends: new Map(), listing: null, again: false, listFailures: 0 };
    this.sync = sync;
    void this.relist();
    return () => { if (this.sync === sync) this.disconnect(); };
  }

  /** Lists the daemon's positions again (a frame said they changed) and merges them by max. */
  relist(): Promise<void> {
    const sync = this.sync;
    if (!sync) return Promise.resolve();
    if (sync.listing) { sync.again = true; return sync.listing; }
    // The pending read will include every invalidation received during this network backoff.
    if (sync.listTimer !== undefined) return Promise.resolve();
    sync.listing = (async () => {
      await this.restored();
      if (this.sync !== sync) return;
      do {
        sync.again = false;
        try {
          const listed = await sync.remote.list();
          if (this.sync !== sync) return;
          sync.listFailures = 0;
          if (sync.listIssue !== undefined) { sync.listIssue = undefined; if (sync.owner === this.owner) this.emit(); }
          this.merge(sync, listed);
        } catch (error) {
          if (this.sync !== sync) return;
          const issue = error instanceof Error ? error.message : String(error);
          if (sync.listIssue !== issue) { sync.listIssue = issue; if (sync.owner === this.owner) this.emit(); }
          // A frame during this failed read must not bypass the unreachable peer's backoff.
          sync.again = false;
          if (listUnreachable(error)) {
            const delay = this.retryDelaysMs[sync.listFailures];
            sync.listFailures = Math.min(sync.listFailures + 1, this.retryDelaysMs.length - 1);
            sync.listTimer = setTimeout(() => {
              sync.listTimer = undefined;
              if (this.sync === sync) void this.relist();
            }, delay);
          }
        }
        // Unsent advances go out even when the list failed: an advance at or below the daemon's value changes nothing.
        for (const conversation of [...this.unsent.get(sync.owner) ?? []]) this.schedule(sync, conversation);
      } while (sync.again);
    })().finally(() => { sync.listing = null; });
    return sync.listing;
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  /** Resolves once every change so far is written. */
  flush(): Promise<void> { return this.writing ?? Promise.resolve(); }

  private emit() { this.revision++; for (const listener of this.listeners) listener(); }
  private restored(): Promise<void> { return this.loaded ??= this.restore(); }

  /** Raises one owner's position to `atMs` when that is newer; whether it moved. */
  private raise(owner: string, conversation: string, atMs: number): boolean {
    const positions = this.all.get(owner) ?? new Map<string, number>();
    const current = positions.get(conversation);
    if (current !== undefined && current >= atMs) return false;
    rememberBounded(positions, conversation, atMs, MAX_CONVERSATIONS);
    rememberBounded(this.all, owner, positions, MAX_OWNERS);
    return true;
  }

  private confirm(sync: Sync, conversation: string, atMs: number): void {
    if ((sync.confirmed.get(conversation) ?? -Infinity) < atMs) sync.confirmed.set(conversation, atMs);
  }

  /** Whether the daemon already holds this device's position, so nothing is left to send. */
  private held(sync: Sync, conversation: string): boolean {
    const local = this.all.get(sync.owner)?.get(conversation), confirmed = sync.confirmed.get(conversation);
    return local === undefined || (confirmed !== undefined && confirmed >= local);
  }

  private settle(owner: string, conversation: string): void {
    const unsent = this.unsent.get(owner);
    if (!unsent?.delete(conversation)) return;
    if (!unsent.size) this.unsent.delete(owner);
    this.save();
  }

  private merge(sync: Sync, listed: readonly ListedReadPosition[]): void {
    let moved = false;
    for (const { conversation, at } of listed) {
      const atMs = ms(at);
      if (atMs === undefined) continue;
      this.confirm(sync, conversation, atMs);
      // The daemon wins only when it is ahead: a smaller value never moves a position back.
      if (this.raise(sync.owner, conversation, atMs)) moved = true;
    }
    for (const conversation of [...this.unsent.get(sync.owner) ?? []]) if (this.held(sync, conversation)) this.settle(sync.owner, conversation);
    if (!moved) return;
    this.save();
    if (sync.owner === this.owner) this.emit();
  }

  /** At most one send in flight per conversation, and at most one per interval. */
  private schedule(sync: Sync, conversation: string): void {
    let send = sync.sends.get(conversation);
    if (!send) { send = { inFlight: false, lastSentAt: -Infinity, failures: 0 }; sync.sends.set(conversation, send); }
    if (send.inFlight || send.timer !== undefined) return;
    const wait = send.lastSentAt + this.everyMs - Date.now();
    if (wait <= 0) { void this.send(sync, conversation, send); return; }
    const scheduled = send;
    scheduled.timer = setTimeout(() => { scheduled.timer = undefined; void this.send(sync, conversation, scheduled); }, wait);
  }

  private async send(sync: Sync, conversation: string, send: Send): Promise<void> {
    if (this.sync !== sync) return;
    const local = this.all.get(sync.owner)?.get(conversation);
    if (local === undefined || this.held(sync, conversation)) { this.settle(sync.owner, conversation); return; }
    send.inFlight = true;
    send.lastSentAt = Date.now();
    let answer: string | undefined;
    try {
      // Advances merge by max, so a fresh idempotency key per send is safe.
      answer = await sync.remote.advance(conversation, new Date(local).toISOString());
    } catch {
      send.inFlight = false;
      if (this.sync !== sync) return;
      const delay = this.retryDelaysMs[Math.min(send.failures, this.retryDelaysMs.length - 1)];
      send.failures++;
      send.timer = setTimeout(() => { send.timer = undefined; this.schedule(sync, conversation); }, delay);
      return;
    }
    send.inFlight = false;
    send.failures = 0;
    if (this.sync !== sync) return;
    // The value the daemon holds after the merge; an older daemon that does not say confirms the one sent.
    const stored = Math.max(ms(answer) ?? local, local);
    this.confirm(sync, conversation, stored);
    if (this.raise(sync.owner, conversation, stored)) { this.save(); if (sync.owner === this.owner) this.emit(); }
    // Read further while this was in flight: that goes next, after the interval.
    if (this.held(sync, conversation)) this.settle(sync.owner, conversation);
    else this.schedule(sync, conversation);
  }

  private disconnect(): void {
    const sync = this.sync;
    if (!sync) return;
    this.sync = undefined;
    clearTimeout(sync.listTimer);
    if (sync.listIssue !== undefined && sync.owner === this.owner) this.emit();
    for (const send of sync.sends.values()) { clearTimeout(send.timer); send.timer = undefined; }
  }

  private async restore(): Promise<void> {
    try {
      const raw = await this.storage.getItem(READ_POSITIONS_KEY);
      const parsed: unknown = raw ? JSON.parse(raw) : undefined;
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
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
      }
    } catch { return; /* unreadable or damaged: start over, nothing is new */ }
    try {
      const raw = await this.storage.getItem(UNSENT_READ_POSITIONS_KEY);
      const parsed: unknown = raw ? JSON.parse(raw) : undefined;
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return;
      for (const [owner, conversations] of Object.entries(parsed as Record<string, unknown>)) {
        const positions = this.all.get(owner);
        if (!positions || !Array.isArray(conversations)) continue;
        const unsent = this.unsent.get(owner) ?? new Set<string>();
        for (const conversation of conversations.slice(-MAX_CONVERSATIONS)) if (typeof conversation === 'string' && positions.has(conversation)) unsent.add(conversation);
        if (unsent.size) this.unsent.set(owner, unsent);
      }
    } catch { /* unsent advances are lost; the next read sends again */ }
  }

  private save(): void {
    this.dirty = true;
    void this.drain();
  }

  private drain(): Promise<void> {
    this.writing ??= (async () => {
      await this.loaded;
      while (this.dirty) {
        this.dirty = false;
        const stored: Stored = Object.fromEntries([...this.all].map(([owner, positions]) => [owner, [...positions]]));
        const unsent = Object.fromEntries([...this.unsent].filter(([owner, conversations]) => conversations.size && this.all.has(owner)).map(([owner, conversations]) => [owner, [...conversations]]));
        try { await this.storage.setItem(READ_POSITIONS_KEY, JSON.stringify(stored)); } catch { /* best-effort */ }
        try { await this.storage.setItem(UNSENT_READ_POSITIONS_KEY, JSON.stringify(unsent)); } catch { /* best-effort */ }
      }
      this.writing = null;
    })();
    return this.writing;
  }
}
