import { actionReceiptCheck, appliedAnswer } from '../../clients/typescript/fractal-client/index.ts';
import { expectedPreparedPacketIdentities } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import type { PreparedPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { markOutboxDispatched, observeOutbox, prepareOutboxRecord, restoreOutboxEntries, restoreOutboxRecord } from '../../clients/typescript/fractal-client/outbox.ts';
import type { OutboxPreparingRecord, OutboxRecord } from '../../clients/typescript/fractal-client/outbox.ts';
import type { ActionResult, EnvelopeOf } from '../../clients/typescript/fractal-client/Models.generated.ts';
import { ConversationAttempts } from './conversationAttempts.ts';
import { messageIntent } from './outboxMessage.ts';
import type { OutboxAccepted, OutboxMessageIntent, OutboxMessageMetadata, OutboxRecoveryRow } from './outboxMessage.ts';
import type { IosOutboxRepository } from './outboxSqlite.ts';
import type { OutboxHash } from './outboxSpool.ts';
import type { Picked } from './images.ts';

export interface OutboxOwner {
  repository: IosOutboxRepository;
  acceptIntent(id: string, text: string, images: readonly Picked[], metadata: OutboxMessageMetadata): Promise<OutboxPreparingRecord>;
  readAttachment(reference: string): Promise<Uint8Array>;
  authorize(): void;
  cleanup(now: number): Promise<void>;
  close(): Promise<void>;
}
export interface OutboxTransport {
  /** Also validates the captured person, actor, profile slot and original connection. */
  authorize(intent: OutboxMessageIntent): void;
  prepare(record: OutboxPreparingRecord, intent: OutboxMessageIntent): Promise<PreparedPacket>;
  post(packet: PreparedPacket): Promise<EnvelopeOf<ActionResult>>;
  /** Direct original-member authoritative read only; absence never proves nonapplication. */
  timeline(record: OutboxRecord<PreparedPacket>, intent: OutboxMessageIntent): Promise<readonly string[]>;
  sha256: OutboxHash;
  errorText(error: unknown): string;
  changed?(): void;
}
type ProjectionResult = { how: 'sent' | 'failed' | 'unconfirmed'; error?: string };

/** Thin process owner. Core owns every certainty transition; SQLite owns acceptance and CAS.
 * The existing ConversationAttempts class is the sole subscription/row projection controller.
 */
export class OutboxRuntime {
  readonly attempts = new ConversationAttempts<OutboxRecoveryRow, ProjectionResult>(() => undefined,
    error => ({ how: 'failed', error: this.transport.errorText(error) }));
  issue = '';
  private readonly busy = new Set<string>();
  private readonly work = new Set<Promise<void>>();
  private readonly notices = new Map<string, string>();
  private stopped = false;
  private restoring?: Promise<void>;
  constructor(readonly owner: OutboxOwner, private readonly transport: OutboxTransport) {}

  private authorize(intent: OutboxMessageIntent): void {
    if (this.stopped) throw new Error('The original outbox connection is no longer available.');
    this.owner.authorize();
    this.transport.authorize(intent);
  }
  private signalIssue(error: unknown): void {
    this.issue = this.transport.errorText(error);
    this.attempts.project([...this.attempts.getSnapshot()]);
  }
  private launch(id: string): void {
    if (this.stopped || this.busy.has(id)) return;
    const operation = this.drive(id);
    this.work.add(operation);
    void operation.finally(() => this.work.delete(operation));
  }
  /** Test/lifecycle barrier, not a request to retry any dispatched record. */
  async settled(): Promise<void> { await Promise.all([...this.work]); }

  async accept(id: string, text: string, images: readonly Picked[], metadata: OutboxMessageMetadata): Promise<OutboxAccepted> {
    const captured: OutboxMessageMetadata = JSON.parse(JSON.stringify(metadata));
    const preview = messageIntent({ text, attachments: [], ...captured });
    if (!preview) throw new Error('Message recovery context is unavailable. Your draft is kept.');
    this.authorize(preview);
    // No discovery, upload, fence, signing or POST is allowed before this barrier.
    await this.owner.acceptIntent(id, text, images, captured);
    this.issue = '';
    await this.refresh();
    this.launch(id);
    return { how: 'accepted', outboxId: id };
  }

  restore = (): Promise<void> => {
    if (this.restoring) return this.restoring;
    const run = this.restoreEntries();
    this.restoring = run;
    void run.finally(() => { if (this.restoring === run) this.restoring = undefined; });
    return run;
  };
  private async restoreEntries(): Promise<void> {
    try {
      const entries = await this.owner.repository.list();
      for (const entry of entries) {
        if (entry._tag !== 'record' || this.busy.has(entry.record.id)) continue;
        const record = entry.record;
        const intent = messageIntent(record.intent);
        if (!intent) continue; // Old packetless/incomplete metadata remains recovery-only.
        try {
          this.authorize(intent);
          const restored = restoreOutboxRecord(record);
          if (restored !== record && !await this.owner.repository.compareAndSwap(record.revision, restored)) continue;
          if (restored.state === 'pending' && !restored.mayHaveDispatched) this.launch(restored.id);
        } catch (error) { this.notices.set(record.id, this.transport.errorText(error)); }
      }
      await this.owner.cleanup(Date.now());
      this.issue = '';
      await this.refresh();
    } catch (error) { this.signalIssue(error); }
  }

  private async drive(id: string): Promise<void> {
    this.busy.add(id);
    try {
      let record = await this.owner.repository.read(id);
      if (!record || record.state !== 'pending' || record.mayHaveDispatched) return;
      const intent = messageIntent(record.intent);
      if (!intent) return;
      this.authorize(intent);
      if (record.phase === 'preparing') {
        const packet = await this.transport.prepare(record, intent);
        this.authorize(intent);
        const identities = await expectedPreparedPacketIdentities(packet, intent.binding.person, this.transport.sha256);
        const ready = prepareOutboxRecord(record, packet, identities);
        if (!ready || !await this.owner.repository.compareAndSwap(record.revision, ready)) return;
        // Dispatch only the persisted packet snapshot, never a mutable preparation object.
        record = await this.owner.repository.read(id);
      }
      if (!record || record.state !== 'pending' || record.phase !== 'ready') return;
      this.authorize(intent);
      const dispatched = markOutboxDispatched(record);
      if (!dispatched || !await this.owner.repository.compareAndSwap(record.revision, dispatched)) return;
      const persisted = await this.owner.repository.read(id);
      if (!persisted || !('packet' in persisted) || persisted.revision !== dispatched.revision || !persisted.mayHaveDispatched) return;
      try {
        // Authorization is deliberately repeated after the durable dispatch marker.
        this.authorize(intent);
        const packet = persisted.packet;
        const expected = { kind: 'message' as const, actionId: packet.id, messageId: persisted.expectedIdentities[0],
          ...(packet.type !== 'message.send' ? { targetId: packet.parameters.target_id } : {}) };
        const response = await actionReceiptCheck(expected)(() => this.transport.post(packet));
        if (response.value.status !== 'completed') throw new Error('Delivery has not been confirmed.');
        const delivered = observeOutbox(persisted, { kind: 'delivered', receipt: { ...response.value, status: 'completed' } });
        await this.owner.repository.compareAndSwap(persisted.revision, delivered);
        this.transport.changed?.();
      } catch (error) {
        const observed = observeOutbox(persisted, { kind: 'failure', applied: appliedAnswer(error) });
        // A failed outcome write cannot restore dispatch permission: the marker is already durable.
        await this.owner.repository.compareAndSwap(persisted.revision, observed);
        this.notices.set(id, this.transport.errorText(error));
      }
    } catch (error) { this.notices.set(id, this.transport.errorText(error)); }
    finally {
      this.busy.delete(id);
      try { await this.refresh(); } catch (error) { this.signalIssue(error); }
    }
  }

  checkStatus = async (id: string): Promise<void> => {
    try {
      const record = await this.owner.repository.read(id);
      if (!record || !('packet' in record)) return;
      const intent = messageIntent(record.intent);
      if (!intent) return;
      this.authorize(intent);
      const identities = await this.transport.timeline(record, intent);
      this.authorize(intent);
      const identity = identities.find(value => record.expectedIdentities.includes(value));
      if (identity) {
        const observed = observeOutbox(record, { kind: 'observed', identity, source: 'timeline' });
        if (observed === record || await this.owner.repository.compareAndSwap(record.revision, observed)) {
          await this.owner.repository.observeTimeline(id, identity);
          this.notices.delete(id);
        }
      } else this.notices.set(id, 'No exact message identity was observed. Delivery remains unknown; absence is not proof.');
      await this.owner.cleanup(Date.now());
      await this.refresh();
    } catch (error) { this.notices.set(id, this.transport.errorText(error)); await this.refresh().catch(error => this.signalIssue(error)); }
  };
  discard = async (id: string): Promise<void> => {
    try {
      await this.owner.repository.discard(id);
      this.notices.delete(id);
      await this.owner.cleanup(Date.now());
      await this.refresh();
    } catch (error) { this.signalIssue(error); }
  };
  private async refresh(): Promise<void> {
    const entries = await this.owner.repository.list();
    const restored = restoreOutboxEntries(entries);
    this.attempts.project(entries.map((entry, index) => {
      const recovery = restored[index];
      if (entry._tag === 'corrupt') {
        return { id: entry.id, phase: 'unconfirmed' as const, draft: { id: entry.id, images: 0, member: this.owner.repository.scope.memberId,
          state: 'corrupt' as const, status: 'storage damaged; delivery unknown', canCheck: false } };
      }
      const record = this.busy.has(entry.record.id) || recovery._tag !== 'record' ? entry.record : recovery.record;
      const intent = messageIntent(record.intent);
      const state = record.state;
      const status = state === 'pending' ? record.phase === 'preparing' ? 'Accepted on this phone; preparing.' : 'Accepted on this phone; sending.'
        : state === 'unknown' ? 'Delivery unknown. The original message will not be sent again.'
        : state === 'not-applied' ? 'Not applied. This attempt will not be sent again.' : 'Delivery confirmed.';
      return { id: record.id, phase: state === 'delivered' ? 'confirmed' as const : state === 'pending' ? 'sending' as const : 'unconfirmed' as const,
        draft: { id: record.id, message: intent?.message, text: record.intent.text, images: record.intent.attachments.length,
          member: this.owner.repository.scope.memberId, state, status: this.notices.get(record.id) ?? (intent ? status : 'Recovery context unavailable; delivery unknown.'),
          canCheck: !!intent && 'packet' in record } };
    }));
  }
  async close(): Promise<void> {
    this.stopped = true;
    await this.settled();
    await this.owner.close();
  }
}
