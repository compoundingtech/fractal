import type { MembershipScope } from './sidebarMemberships';

export const SIDEBAR_JOURNAL_VERSION = 1;
export type JournalStatus = 'Intent' | 'AwaitingPersistence' | 'Sending' | 'UnknownDelivery' | 'AwaitingObservation' | 'AwaitingCompletionPersistence' | 'AwaitingFreshBase' | 'Observed' | 'Refused' | 'PartiallyApplied';
export type JournalEffect = 'PersistPending' | 'Send' | 'ReadBase' | 'PersistCompletion' | 'PersistRemoval' | 'Replan';
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
  storageFailed(): TEvent;
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
 * One instance owns one public connection/scope. All mutation must go through dispatch;
 * effect callbacks must not await a nested dispatch on this serialized queue. */
export class SidebarJournal<TState extends JournalState, TEvent> {
  private pending: Promise<unknown> = Promise.resolve();
  private readonly key: string;
  constructor(private readonly storage: JournalStorage, private readonly scope: MembershipScope, private readonly reducer: JournalReducer<TState, TEvent>, private readonly run: (effect: 'Send' | 'ReadBase' | 'Replan', state: TState) => Promise<void>) {
    this.key = `st3.sidebar.journal.v1:${JSON.stringify([scope.owner, scope.container, scope.endpoint, scope.host, scope.projection])}`;
  }
  private serial<TResult>(work: () => Promise<TResult>): Promise<TResult> {
    const next = this.pending.then(work); this.pending = next.catch(() => {}); return next;
  }
  restore(): Promise<JournalRestore<TState>> {
    return this.serial(async () => {
      const raw = await this.storage.getItem(this.key);
      if (raw === null) return { empty: true };
      try {
        const record: unknown = JSON.parse(raw);
        if (!record || typeof record !== 'object' || !('version' in record)) return { error: { type: 'InvalidRecord' } };
        // Refuse unknown versions without invoking the current state decoder.
        if (record.version !== SIDEBAR_JOURNAL_VERSION) return { error: { type: 'Version', version: record.version } };
        if (!('state' in record) || Object.keys(record).length !== 2) return { error: { type: 'InvalidRecord' } };
        const state = this.reducer.decode(record.state);
        if (!state) return { error: { type: 'InvalidRecord' } };
        if (!sameScope(state.scope, this.scope) || state.request && !sameScope(state.request.fence.scope, this.scope)) return { error: { type: 'ScopeMismatch' } };
        return { state, effects: recover(state) };
      } catch { return { error: { type: 'InvalidRecord' } }; }
    });
  }
  dispatch(state: TState, event: TEvent): Promise<void> {
    return this.serial(() => this.execute(state, this.reducer.reduce(state, event)));
  }
  /** Execute restored effects before accepting new edit events. Restore itself never sends. */
  resume(restored: { state: TState; effects: JournalEffect[] }): Promise<void> {
    return this.serial(() => this.execute(restored.state, restored.effects));
  }
  private async execute(state: TState, effects: JournalEffect[]): Promise<void> {
    for (const effect of effects) {
      if (effect === 'Send' || effect === 'ReadBase' || effect === 'Replan') { await this.run(effect, state); continue; }
      if (!sameScope(state.scope, this.scope) || state.request && !sameScope(state.request.fence.scope, this.scope)) throw new Error('Sidebar journal scope mismatch');
      let ack: TEvent;
      try {
        if (effect === 'PersistRemoval') { await this.storage.removeItem(this.key); continue; }
        // Capture bytes AND acknowledgement before yielding to the storage operation.
        const raw = JSON.stringify({ version: SIDEBAR_JOURNAL_VERSION, state: this.reducer.encode(state) });
        ack = this.reducer.persisted(state, effect === 'PersistCompletion');
        await this.storage.setItem(this.key, raw);
      } catch (error) {
        // Do not recursively retry persistence effects from StorageFailed or send past this barrier.
        this.reducer.reduce(state, this.reducer.storageFailed());
        throw error;
      }
      await this.execute(state, this.reducer.reduce(state, ack));
    }
  }
}
