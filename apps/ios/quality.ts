import { SloCounters, isQualityHour, mergeQualityHour, retainQualityHours, sloHour, type QualityHour } from '../../clients/typescript/st3-views/slo';

export const QUALITY_HISTORY_KEY = 'st3.quality.hours.v1';
export const QUALITY_RUN_KEY = 'st3.quality.run.v1';
export const QUALITY_MAX_BYTES = 2 * 1024 * 1024;
export type QualityStorage = { getItem(key: string): Promise<string | null>; setItem(key: string, value: string): Promise<void> };
export type QualityClock = { now(): number; wall(): number; after(fn: () => void, ms: number): unknown; cancel(timer: unknown): void; frame(fn: () => void): void };
export function readQualityLaunchClock(perf: { rnStartupTiming?: { startTime: number | null } }): number | null {
  try { return perf.rnStartupTiming?.startTime ?? null; } catch { return null; }
}
type Answer = { token: number; at: number; accepted: boolean; gone: boolean; seen: boolean };

/** Separate from Observations: this recorder has no client, fetch, reporter or graph port. */
export class LocalQuality {
  readonly counters = new SloCounters();
  readonly ready: Promise<void>;
  wasUnclean = false;
  private history: QualityHour[] = [];
  private tail: Promise<void>;
  private loaded = false;
  private active: boolean;
  private accounted: number;
  private window: number;
  private timer: unknown;
  private liveValue = false;
  private everLive = false;
  private reconnectAt = -1;
  private first = false;
  private real = false;
  private top = -1;
  private placeholders = -1;
  private receivedWindows = 0;
  private readonly started: number;
  private readonly keys = new Float64Array(64);
  private keyCount = 0;
  private readonly frames = new Float64Array(128);
  private readonly kinds = new Uint8Array(128);
  private frameCount = 0;
  private committedFrames = 0;
  private committedKeys = 0;
  private shown = 0;
  private settled = false;
  private nextTop = 0;
  private nextPlaceholders = 3;
  private paintPending = false;
  private paintPhase = 0;
  private conversationToken = 0;
  private conversationAt = -1;
  private conversationReady = false;
  private terminalToken = 0;
  private terminalAt = -1;
  private terminalReady = false;
  private terminalAttached = false;
  private sequence = 0;
  private readonly answers = new Map<string, Answer>();
  private readonly landed = new Float64Array(256);
  private landedCount = 0;
  private committedLanded = 0;
  private readonly listeners = new Set<() => void>();
  private readonly paint: () => void;
  private readonly boundary: () => void;

  constructor(private readonly storage: QualityStorage, private readonly clock: QualityClock, private readonly host: string, private readonly build: string, active = true, launchStarted: number | null = clock.now()) {
    this.active = active;
    this.accounted = clock.now();
    this.started = launchStarted !== null && Number.isFinite(launchStarted) && launchStarted <= this.accounted ? launchStarted : NaN;
    if (Number.isNaN(this.started)) this.counters.add('measurement_overflow');
    this.window = sloHour(clock.wall());
    // Reusable callbacks; key/frame/presentation accounting uses only fixed numeric storage.
    this.paint = () => {
      if (!this.active) { this.paintPending = false; return; }
      if (this.paintPhase++ === 0) { this.clock.frame(this.paint); return; }
      this.paintPending = false;
      this.presented();
    };
    this.boundary = () => { void this.flush(); this.scheduleHour(); };
    this.counters.add('launches');
    this.ready = this.load();
    this.tail = this.ready;
  }

  private async load(): Promise<void> {
    const raw = await this.read(QUALITY_HISTORY_KEY);
    if (raw && raw.length <= QUALITY_MAX_BYTES) {
      // Parse each line once; one indexed merge and one sort, independent of
      // how many older rows exist. Startup must not repeatedly rebuild history.
      const hours = new Map<number, QualityHour>();
      const current = sloHour(this.clock.wall());
      for (const text of raw.split('\n').slice(-336)) {
        try {
          const row: unknown = JSON.parse(text);
          if (!isQualityHour(row) || row.window_start < current - 335 * 3600 || row.window_start > current) continue;
          const previous = hours.get(row.window_start);
          hours.set(row.window_start, previous ? mergeQualityHour(previous, row) : row);
        } catch { /* incomplete local line */ }
      }
      this.history = [...hours.values()].sort((a, b) => a.window_start - b.window_start);
    }
    const mark = await this.read(QUALITY_RUN_KEY);
    this.wasUnclean = mark === 'active' || mark === 'panic';
    if (mark === 'active') this.counters.add('silent_exits');
    this.loaded = true;
    await this.write(QUALITY_RUN_KEY, this.active ? 'active' : 'clean');
    if (this.active) this.scheduleHour();
    this.changed();
  }
  private async read(key: string): Promise<string | null> { try { return await this.storage.getItem(key); } catch { return null; } }
  private async write(key: string, value: string): Promise<void> { try { await this.storage.setItem(key, value); } catch { /* local diagnostics must not surface a storage failure */ } }
  private queue(task: () => Promise<void>): Promise<void> { this.tail = this.tail.then(task); return this.tail; }
  private accrue(): void {
    const now = this.clock.now(), elapsed = Math.max(0, now - this.accounted);
    if (this.active) {
      this.counters.add('active_ms', elapsed);
      if (!this.liveValue) this.counters.add('not_live_ms', elapsed);
    }
    this.accounted = now;
  }
  private scheduleHour(): void {
    this.clock.cancel(this.timer);
    this.timer = undefined;
    if (this.active && this.loaded) this.timer = this.clock.after(this.boundary, Math.max(1, (sloHour(this.clock.wall()) + 3600) * 1000 - this.clock.wall()));
  }
  flush(): Promise<void> {
    this.accrue();
    const line = this.counters.line(this.window, this.host, this.build, true);
    this.window = sloHour(this.clock.wall());
    return this.queue(async () => {
      this.history = retainQualityHours(this.history, line);
      await this.write(QUALITY_HISTORY_KEY, this.history.map(row => JSON.stringify(row)).join('\n'));
      this.changed();
    });
  }
  foreground(active: boolean): void {
    if (active === this.active) return;
    this.accrue();
    this.active = active;
    this.liveValue = false;
    this.reconnectAt = active && this.everLive ? this.clock.now() : -1;
    this.keyCount = this.frameCount = this.landedCount = 0;
    this.committedKeys = this.committedFrames = this.committedLanded = 0;
    if (active) {
      this.window = sloHour(this.clock.wall());
      this.schedulePaint();
      void this.queue(() => this.write(QUALITY_RUN_KEY, 'active'));
    } else {
      this.conversationAt = this.terminalAt = -1; this.answers.clear();
      void this.flush();
      void this.queue(() => this.write(QUALITY_RUN_KEY, 'clean'));
    }
    this.scheduleHour();
  }
  /** Native crashes/OS kills are inferred from a stale foreground marker. Fatal JS errors count here. */
  panic(): void {
    this.counters.add('panics');
    void this.flush();
    void this.queue(() => this.write(QUALITY_RUN_KEY, 'panic'));
  }
  live(value: boolean): void {
    if (value === this.liveValue) return;
    this.accrue();
    this.liveValue = value;
    if (!value && this.active && this.everLive && this.reconnectAt < 0) this.reconnectAt = this.clock.now();
  }
  get loadingMask(): number { return 3 ^ this.receivedWindows; }
  get windowsReady(): boolean { return this.receivedWindows === 3; }
  resetWindows(): void { this.receivedWindows = 0; }
  received(kind: 1 | 2, at: number): void {
    this.receivedWindows |= kind;
    if (!this.active) return;
    if (this.frameCount === this.frames.length) { this.counters.add('measurement_overflow'); return; }
    this.frames[this.frameCount] = at; this.kinds[this.frameCount++] = kind;
  }
  key(): void {
    if (!this.active) return;
    if (this.keyCount === this.keys.length) { this.counters.add('measurement_overflow'); return; }
    this.keys[this.keyCount++] = this.clock.now();
    this.committedKeys = this.keyCount; this.schedulePaint();
  }
  /** A React commit stages the visible windows; two animation callbacks include a native paint opportunity.
   * Frames arriving after this commit remain pending for the commit that contains them. */
  commit(shown = this.shown, settled = this.settled, top = this.nextTop, placeholders = this.nextPlaceholders): void {
    this.shown = shown; this.settled = settled; this.nextTop = top; this.nextPlaceholders = placeholders;
    this.committedFrames = this.frameCount; this.committedKeys = this.keyCount; this.committedLanded = this.landedCount;
    this.schedulePaint();
  }
  private schedulePaint(): void {
    if (!this.active || this.paintPending) return;
    this.paintPending = true; this.paintPhase = 0; this.clock.frame(this.paint);
  }
  private presented(): void {
    const now = this.clock.now();
    if (!this.first) { this.first = true; this.counters.observe('first_frame_ms', now - this.started); }
    if (!this.real) {
      if (this.top >= 0 && this.top !== this.nextTop) this.counters.add('first_screen_top_bar_flips');
      if (this.placeholders >= 0 && this.placeholders !== this.nextPlaceholders) this.counters.add('first_screen_placeholder_flips');
      this.top = this.nextTop; this.placeholders = this.nextPlaceholders;
      if (this.settled) { this.real = true; this.counters.observe('first_real_state_ms', now - this.started); }
    }
    for (let i = 0; i < this.committedKeys; i++) this.counters.observe('key_to_redraw_ms', now - this.keys[i]);
    this.keys.copyWithin(0, this.committedKeys, this.keyCount); this.keyCount -= this.committedKeys;
    for (let i = 0; i < this.committedFrames; i++) if (this.kinds[i] & this.shown) this.counters.observe('freshness_ms', now - this.frames[i]);
    this.frames.copyWithin(0, this.committedFrames, this.frameCount); this.kinds.copyWithin(0, this.committedFrames, this.frameCount); this.frameCount -= this.committedFrames;
    if (this.conversationAt >= 0 && this.conversationReady) { this.counters.observe('conversation_open_ms', now - this.conversationAt); this.conversationAt = -1; }
    if (this.terminalAt >= 0 && this.terminalReady && this.terminalAttached) { this.counters.observe('terminal_open_ms', now - this.terminalAt); this.terminalAt = -1; }
    if (this.shown & 1) {
      for (let i = 0; i < this.committedLanded; i++) this.counters.observe('alert_landed_ms', now - this.landed[i]);
      this.landed.copyWithin(0, this.committedLanded, this.landedCount); this.landedCount -= this.committedLanded;
    }
    this.committedKeys = this.committedFrames = this.committedLanded = 0;
    if (this.liveValue) {
      if (this.reconnectAt >= 0) this.counters.observe('reconnect_ms', now - this.reconnectAt);
      this.reconnectAt = -1; this.everLive = true;
    }
  }
  beginConversation(): number {
    this.counters.add('conversation_opens');
    this.conversationAt = this.clock.now(); this.conversationReady = false;
    return this.conversationToken = ++this.sequence;
  }
  conversationVisible(token: number): void { if (token === this.conversationToken) { this.conversationReady = true; this.schedulePaint(); } }
  endConversation(token: number): void { if (token === this.conversationToken) { this.conversationAt = -1; this.conversationReady = false; } }
  conversationError(token: number, code?: string): void {
    if (token !== this.conversationToken || this.conversationAt < 0) return;
    this.counters.add(code === 'transcript-not-bound' ? 'open_not_bound' : code === 'remote-unavailable' ? 'open_remote_unavailable' : 'open_cannot_load');
    this.conversationAt = -1;
  }
  beginTerminal(): number { this.terminalAt = this.clock.now(); this.terminalReady = this.terminalAttached = false; return this.terminalToken = ++this.sequence; }
  terminalVisible(token: number): void { if (token === this.terminalToken) { this.terminalReady = true; this.schedulePaint(); } }
  terminalAttachment(token: number): void { if (token === this.terminalToken) { this.terminalAttached = true; this.schedulePaint(); } }
  endTerminal(token: number): void { if (token === this.terminalToken) { this.terminalAt = -1; this.terminalReady = false; } }
  answerSent(id: string): number {
    if (!this.answers.has(id) && this.answers.size === 256) { this.counters.add('measurement_overflow'); return 0; }
    const token = ++this.sequence;
    this.answers.set(id, { token, at: this.clock.now(), accepted: false, gone: false, seen: false });
    return token;
  }
  answerConfirmed(id: string, token: number, accepted: boolean): void {
    const answer = this.answers.get(id);
    if (!answer || answer.token !== token) return;
    if (!accepted) { this.answers.delete(id); return; }
    answer.accepted = true; this.keepLanded(id, answer);
  }
  attentionWindow(items: readonly { id: string; state: string }[], truncated: boolean): void {
    if (truncated) return;
    for (const answer of this.answers.values()) answer.seen = false;
    for (const item of items) { const answer = this.answers.get(item.id); if (answer) answer.seen = item.state === 'open'; }
    for (const [id, answer] of this.answers) { if (!answer.seen) answer.gone = true; this.keepLanded(id, answer); }
  }
  private keepLanded(id: string, answer: Answer): void {
    if (!answer.accepted || !answer.gone) return;
    if (this.landedCount === this.landed.length) this.counters.add('measurement_overflow');
    else this.landed[this.landedCount++] = answer.at;
    this.answers.delete(id);
    this.committedLanded = this.landedCount; this.schedulePaint();
  }
  subscribe(changed: () => void): () => void { this.listeners.add(changed); return () => { this.listeners.delete(changed); }; }
  private changed(): void { for (const listener of this.listeners) listener(); }
  /** Fleet reads completed clock hours on focus and when an hour is written. */
  report(): { last: QualityHour | null; day: QualityHour | null } {
    const current = sloHour(this.clock.wall());
    const rows = this.history.filter(row => row.window_start >= current - 24 * 3600 && row.window_start < current);
    const last = rows.find(row => row.window_start === current - 3600) ?? null;
    const combined = rows.length ? rows.slice(1).reduce(mergeQualityHour, rows[0]) : null;
    const day = combined ? { ...combined, targets: rows[rows.length - 1].targets } : null;
    return { last, day };
  }
}
