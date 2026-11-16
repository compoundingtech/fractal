import type { AnswerOutcome } from './answer';
export type AttentionDelivery = 'pending' | 'confirmed' | 'settled' | 'refused' | 'unconfirmed';
/** One connection's in-memory delivery ownership. A lost reply never authorizes a new answer. */
export class AttentionDeliveryTracker {
  private listeners = new Set<() => void>();
  private writes = new Set<string>();
  private checks = new Map<string, () => Promise<boolean>>();
  private alive = true;
  private revisions: Record<string, string> = {};
  private state: { pending: ReadonlySet<string>; delivery: Readonly<Record<string, AttentionDelivery>>; revisions: Readonly<Record<string, string>> } = { pending: new Set(), delivery: {}, revisions: {} };
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  snapshot = () => this.state;
  open() { this.alive = true; }
  isOpen() { return this.alive; }
  close() { this.alive = false; this.checks.clear(); this.listeners.clear(); }
  private publish(id: string, delivery: AttentionDelivery) {
    if (!this.alive) return;
    this.state = { pending: new Set(this.writes), delivery: { ...this.state.delivery, [id]: delivery }, revisions: { ...this.revisions } };
    for (const listener of this.listeners) listener();
  }
  async run(id: string, write: () => Promise<boolean>, checking = false, revision?: string): Promise<boolean> {
    if (!this.alive || this.writes.has(id) || (!checking && this.checks.has(id))) return false;
    if (!checking && revision !== undefined) this.revisions[id] = revision;
    this.writes.add(id); this.publish(id, 'pending');
    try {
      const result = await write();
      if (this.state.delivery[id] === 'pending') this.publish(id, result ? 'confirmed' : 'refused');
      return result;
    } catch (error) { if (this.state.delivery[id] === 'pending') this.publish(id, 'refused'); throw error; } finally {
      this.writes.delete(id);
      if (this.alive) { this.state = { ...this.state, pending: new Set(this.writes) }; for (const listener of this.listeners) listener(); }
    }
  }
  accept(id: string, outcome: AnswerOutcome, confirmed: () => void | Promise<void> = () => {}): Promise<boolean> {
    if (!this.alive) return Promise.resolve(false);
    if (outcome.ok) { this.checks.delete(id); this.publish(id, outcome.how === 'already' ? 'settled' : 'confirmed'); return outcome.how === 'already' ? Promise.resolve(true) : Promise.resolve(confirmed()).then(() => true); }
    if (outcome.how === 'unconfirmed') this.checks.set(id, async () => this.accept(id, await outcome.check(), confirmed));
    else this.checks.delete(id);
    this.publish(id, outcome.how); return Promise.resolve(false);
  }
  forRevision(id: string, revision?: string): AttentionDelivery | undefined {
    const delivery = this.state.delivery[id];
    return revision !== undefined && (delivery === 'confirmed' || delivery === 'settled' || delivery === 'refused') && this.state.revisions[id] !== revision ? undefined : delivery;
  }
  check(id: string): Promise<boolean> {
    const check = this.checks.get(id);
    return check ? this.run(id, check, true) : Promise.resolve(false);
  }
}

/** Capability absence during socket recovery does not change the original client/principal. */
export function retainDeliveryPrincipal<C>(previous: { client: C; actor: string | undefined }, client: C, discoveredActor: string | undefined): { client: C; actor: string | undefined } {
  return { client, actor: discoveredActor ?? (previous.client === client ? previous.actor : undefined) };
}
