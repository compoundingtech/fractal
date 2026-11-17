export type AttemptOutcome = { how: 'sent' | 'failed' | 'unconfirmed' };
export type ConversationAttempt<TDraft, TResult extends AttemptOutcome> = {
  id: string;
  draft: TDraft;
  phase: 'sending' | 'failed' | 'unconfirmed' | 'confirmed';
  result?: TResult;
  retry?: () => Promise<TResult>;
};

/** Process-lifetime ownership only. Adapters retain their exact bound request/receipt ports.
 * This neither serializes packets nor decides application certainty: delivery.ts does that.
 */
export class ConversationAttempts<TDraft, TResult extends AttemptOutcome> {
  private rows: readonly ConversationAttempt<TDraft, TResult>[] = [];
  private readonly listeners = new Set<() => void>();
  constructor(private readonly retryOf: (result: TResult) => (() => Promise<TResult>) | undefined,
    private readonly failed: (error: unknown) => TResult) {}
  getSnapshot = (): readonly ConversationAttempt<TDraft, TResult>[] => this.rows;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private publish = (rows: readonly ConversationAttempt<TDraft, TResult>[]): void => {
    this.rows = rows;
    for (const listener of this.listeners) listener();
  };
  remove = (id: string): void => { this.publish(this.rows.filter(row => row.id !== id)); };
  retain = (keep: (attempt: ConversationAttempt<TDraft, TResult>) => boolean): void => { this.publish(this.rows.filter(keep)); };
  has = (id: string): boolean => this.rows.some(row => row.id === id);
  async send(id: string, draft: TDraft, operation: () => Promise<TResult>, onSettled?: (result: TResult) => void): Promise<TResult> {
    if (this.has(id)) throw new Error('Resolve the earlier send before starting another.');
    const initial: ConversationAttempt<TDraft, TResult> = { id, draft, phase: 'sending' };
    this.publish([...this.rows, initial]);
    return this.run(initial, operation, onSettled);
  }
  /** Explicit ordinary-chat Send as new. Refusing it does not settle the earlier unknown. */
  async sendAsNew(id: string, operation: () => Promise<TResult>): Promise<TResult> {
    const previous = this.rows.find(row => row.id === id);
    if (!previous || previous.phase === 'sending') throw new Error('The original attempt is unavailable.');
    const result = await this.run(previous, operation);
    if (result.how === 'failed') this.publish(this.rows.map(row => row.id === id ? previous : row));
    return result;
  }
  private async run(previous: ConversationAttempt<TDraft, TResult>, operation: () => Promise<TResult>, onSettled?: (result: TResult) => void): Promise<TResult> {
    const running: ConversationAttempt<TDraft, TResult> = { ...previous, phase: 'sending', retry: undefined };
    this.publish(this.rows.map(row => row.id === previous.id ? running : row));
    let result: TResult;
    try { result = await operation(); }
    catch (error) { result = this.failed(error); }
    const original = this.retryOf(result);
    const retry = original ? () => this.rows.find(row => row.id === previous.id)?.retry === retry
      ? this.run(settled, original, onSettled) : Promise.resolve(result) : undefined;
    const settled: ConversationAttempt<TDraft, TResult> = { id: previous.id, draft: previous.draft,
      phase: result.how === 'sent' ? 'confirmed' : result.how, result, retry };
    // A detached/reset owner cannot recreate an attempt removed while transport was pending.
    this.publish(this.rows.map(row => row.id === previous.id ? settled : row));
    onSettled?.(result);
    return result;
  }
}
