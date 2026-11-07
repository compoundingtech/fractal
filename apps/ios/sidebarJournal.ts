import type { MembershipScope } from './sidebarMemberships';

export const SIDEBAR_JOURNAL_VERSION = 1;
export type JournalStatus = 'Intent' | 'AwaitingPersistence' | 'Sending' | 'UnknownDelivery' | 'AwaitingObservation' | 'AwaitingCompletionPersistence' | 'AwaitingFreshBase' | 'Observed' | 'Refused' | 'PartiallyApplied';
export type WriteDelivery = 'First' | 'Replay';
export type JournalEffect = 'PersistPending' | { type: 'Send'; delivery: WriteDelivery } | 'ReadBase' | 'PersistCompletion' | 'PersistRemoval' | 'Replan';
export type JournalPersistence = 'Pending' | 'Completion' | 'Removal';
export type JournalState = { scope: MembershipScope; status: JournalStatus; request: { fence: { scope: MembershipScope } } | null };
export type JournalStorage = { getItem(key: string): Promise<string | null>; setItem(key: string, value: string): Promise<void>; removeItem(key: string): Promise<void> };
export type JournalRestoreError = { type: 'Version'; version: unknown } | { type: 'InvalidRecord' } | { type: 'ScopeMismatch' };
export type JournalRestore<TState> = { state: TState; effects: JournalEffect[] } | { error: JournalRestoreError } | { empty: true };
/** PR 24 supplies the concrete edit reducer/codec. Decode validates all durable fields;
 * encode includes only intent, identities, fences and lifecycle state, never transport credentials.
 * Persistence acknowledgements must carry the captured request identity. */
export type JournalReducer<TState extends JournalState, TEvent> = {
  encode(state: TState): unknown;
  decode(value: unknown): TState | undefined;
  reduce(state: TState, event: TEvent): JournalEffect[];
  persisted(state: TState, completion: boolean): TEvent;
  storageFailed(state: TState, phase: JournalPersistence): TEvent;
};
const sameScope = (a: MembershipScope, b: MembershipScope): boolean => a.owner === b.owner && a.container === b.container && a.endpoint === b.endpoint && a.host === b.host && a.projection === b.projection;
const recover = (state: JournalState): JournalEffect[] => {
  switch (state.status) {
    case 'AwaitingPersistence': case 'Sending': case 'UnknownDelivery': state.status = 'AwaitingPersistence'; return ['PersistPending'];
    case 'AwaitingCompletionPersistence': return ['PersistCompletion'];
    case 'Observed': return ['PersistRemoval'];
    case 'Intent': return ['ReadBase', 'Replan'];
    default: return ['ReadBase'];
  }
};
/** Unwired AsyncStorage host, following LocalDrafts' injected storage precedent.
 * One instance owns one public connection/scope; await close() before replacing that connection.
 * Callbacks must honor signal and must not await a nested dispatch on this serialized queue. */
export class SidebarJournal<TState extends JournalState, TEvent> {
  private pending: Promise<unknown> = Promise.resolve();
  private readonly key: string;
  private readonly lifetime = new AbortController();
  private queued = 0;
  private restored?: TState;
  private storagePending: Promise<void> = Promise.resolve();
  constructor(private readonly storage: JournalStorage, private readonly scope: MembershipScope, private readonly reducer: JournalReducer<TState, TEvent>, private readonly run: (effect: Extract<JournalEffect, { type: 'Send' }> | 'ReadBase' | 'Replan', state: TState, signal: AbortSignal) => Promise<void>, private readonly limits = { maxQueued: 64, timeoutMs: 30000 }) {
    if (!Number.isSafeInteger(limits.maxQueued) || limits.maxQueued < 1 || !Number.isSafeInteger(limits.timeoutMs) || limits.timeoutMs < 1) throw new Error('Invalid sidebar journal limits');
    this.key = `st3.sidebar.journal.v1:${JSON.stringify([scope.owner, scope.container, scope.endpoint, scope.host, scope.projection])}`;
  }
  /** Owners must await this storage barrier before creating a replacement host for this scope. */
  close(): Promise<void> { this.lifetime.abort(); return this.storagePending; }
  private storageOperation<TResult>(work: () => Promise<TResult>): Promise<TResult> {
    const operation = Promise.resolve().then(() => { this.assertOpen(); return work(); });
    this.storagePending = operation.then(() => {}, () => {});
    return operation;
  }
  private assertOpen(): void { if (this.lifetime.signal.aborted) throw new Error('Sidebar journal closed'); }
  private serial<TResult>(work: () => Promise<TResult>): Promise<TResult> {
    if (this.lifetime.signal.aborted) return Promise.reject(new Error('Sidebar journal closed'));
    if (this.queued >= this.limits.maxQueued) return Promise.reject(new Error('Sidebar journal queue full'));
    this.queued++;
    const next = this.pending.then(async () => {
      this.assertOpen();
      const timer = setTimeout(() => { void this.close(); }, this.limits.timeoutMs);
      let onAbort = () => {};
      try {
        const stopped = new Promise<never>((_, reject) => {
          onAbort = () => reject(new Error('Sidebar journal closed'));
          this.lifetime.signal.addEventListener('abort', onAbort, { once: true });
        });
        return await Promise.race([work(), stopped]);
      } finally { clearTimeout(timer); this.lifetime.signal.removeEventListener('abort', onAbort); }
    });
    this.pending = next.catch(() => {});
    return next.finally(() => { this.queued--; });
  }
  restore(): Promise<JournalRestore<TState>> {
    return this.serial(async () => {
      const raw = await this.storageOperation(() => this.storage.getItem(this.key));
      this.assertOpen();
      if (raw === null) return { empty: true };
      try {
        const record: unknown = JSON.parse(raw);
        if (!record || typeof record !== 'object' || !('version' in record)) return { error: { type: 'InvalidRecord' } };
        if (typeof record.version !== 'number' || !Number.isInteger(record.version) || record.version < 0 || record.version > 0xffffffff) return { error: { type: 'InvalidRecord' } };
        // Refuse unknown versions without invoking the current state decoder.
        if (record.version !== SIDEBAR_JOURNAL_VERSION) return { error: { type: 'Version', version: record.version } };
        if (!('state' in record) || Object.keys(record).length !== 2) return { error: { type: 'InvalidRecord' } };
        const state = this.reducer.decode(record.state);
        if (!state) return { error: { type: 'InvalidRecord' } };
        if (!sameScope(state.scope, this.scope) || state.request && !sameScope(state.request.fence.scope, this.scope)) return { error: { type: 'ScopeMismatch' } };
        this.restored = state;
        return { state, effects: recover(state) };
      } catch { return { error: { type: 'InvalidRecord' } }; }
    });
  }
  dispatch(state: TState, event: TEvent): Promise<void> {
    return this.serial(() => {
      if (this.restored) throw new Error('Resume restored sidebar journal first');
      return this.execute(state, this.reducer.reduce(state, event));
    });
  }
  /** Execute restored effects before accepting new edit events. Restore itself never sends. */
  resume(restored: { state: TState; effects: JournalEffect[] }): Promise<void> {
    return this.serial(async () => {
      if (this.restored && this.restored !== restored.state) throw new Error('Sidebar journal restore identity mismatch');
      await this.execute(restored.state, restored.effects);
      this.restored = undefined;
    });
  }
  private async execute(state: TState, effects: JournalEffect[]): Promise<void> {
    for (const effect of effects) {
      this.assertOpen();
      if (!sameScope(state.scope, this.scope) || state.request && !sameScope(state.request.fence.scope, this.scope)) throw new Error('Sidebar journal scope mismatch');
      if (typeof effect === 'object' || effect === 'ReadBase' || effect === 'Replan') { await this.run(effect, state, this.lifetime.signal); this.assertOpen(); continue; }
      let ack: TEvent;
      const failure = this.reducer.storageFailed(state, effect === 'PersistCompletion' ? 'Completion' : effect === 'PersistRemoval' ? 'Removal' : 'Pending');
      try {
        if (effect === 'PersistRemoval') { await this.storageOperation(() => this.storage.removeItem(this.key)); if (this.lifetime.signal.aborted) return; continue; }
        // Capture bytes AND acknowledgement before yielding to the storage operation.
        const raw = JSON.stringify({ version: SIDEBAR_JOURNAL_VERSION, state: this.reducer.encode(state) });
        ack = this.reducer.persisted(state, effect === 'PersistCompletion');
        await this.storageOperation(() => this.storage.setItem(this.key, raw));
      } catch (error) {
        // Do not recursively retry persistence effects from StorageFailed or send past this barrier.
        if (this.lifetime.signal.aborted) return;
        this.reducer.reduce(state, failure);
        throw error;
      }
      if (this.lifetime.signal.aborted) return;
      await this.execute(state, this.reducer.reduce(state, ack));
    }
  }
}
