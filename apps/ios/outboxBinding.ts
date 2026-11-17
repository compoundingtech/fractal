import { outboxDeploymentId, restoreOutboxRecord, restoreOutboxEntries } from '../../clients/typescript/fractal-client/outbox.ts';
import type { OutboxRecord, OutboxRepository, OutboxScope } from '../../clients/typescript/fractal-client/outbox.ts';
import type { PairingIssuer } from './pairingProof.ts';

export type OutboxProfileIdentity = {
  profileId?: string;
  pairingIssuer?: PairingIssuer;
  outboxNoticeSeen?: boolean;
};
export const REPAIR_OUTBOX_NOTICE = 'Re-pair to enable crash-safe sending.';
export type OutboxProfileMode =
  | { mode: 'bound'; scope: OutboxScope; authorized: boolean; notice: typeof REPAIR_OUTBOX_NOTICE | undefined }
  | { mode: 'unprotected'; notice: typeof REPAIR_OUTBOX_NOTICE | undefined };

/** Legacy mode never authorizes a durable record; existing nonpersistent sends stay available. */
export const outboxProfileMode = (profile: OutboxProfileIdentity, memberId: string, capabilities?: { issuer?: string; issuer_key?: string } | null): OutboxProfileMode => {
  const issuer = profile.pairingIssuer;
  if (!profile.profileId || !memberId || !issuer?.issuer.startsWith('host/') || !issuer.issuerKey) {
    return { mode: 'unprotected', notice: profile.outboxNoticeSeen ? undefined : REPAIR_OUTBOX_NOTICE };
  }
  const authorized = capabilities?.issuer === issuer.issuer && capabilities?.issuer_key === issuer.issuerKey;
  return { mode: 'bound', scope: { principalId: profile.profileId, memberId, deploymentId: outboxDeploymentId(issuer.issuer, issuer.issuerKey) },
    authorized, notice: authorized || profile.outboxNoticeSeen ? undefined : REPAIR_OUTBOX_NOTICE };
};

/** Caller persists this metadata with the profile; acknowledgement confers no binding. */
export const acknowledgeOutboxNotice = <TProfile extends OutboxProfileIdentity>(profile: TProfile): TProfile => ({
  ...profile, outboxNoticeSeen: true,
});

export const sameOutboxScope = (left: OutboxScope, right: OutboxScope): boolean =>
  left.principalId === right.principalId && left.memberId === right.memberId && left.deploymentId === right.deploymentId;

export class OutboxBindingUnavailableError extends Error {
  readonly state = 'unknown';
  constructor() { super(REPAIR_OUTBOX_NOTICE); this.name = 'OutboxBindingUnavailableError'; }
}

export const requireOutboxAuthorization = (mode: OutboxProfileMode, scope: OutboxScope): void => {
  if (mode.mode !== 'bound' || !mode.authorized || !sameOutboxScope(mode.scope, scope)) throw new OutboxBindingUnavailableError();
};

/** Visibility is scoped by the persisted identity, never by a newly reported issuer.
 * A capabilities mismatch does not hide, migrate, sign or dispatch these entries.
 */
export const restoreBoundEntries = async <TPacket>(repository: OutboxRepository<TPacket>, mode: OutboxProfileMode) => {
  if (mode.mode !== 'bound' || !sameOutboxScope(repository.scope, mode.scope)) throw new OutboxBindingUnavailableError();
  return restoreOutboxEntries(await repository.list());
};

export type BoundAttempt<TPacket> =
  | { state: 'unknown'; reason: 'binding-unavailable' | 'binding-mismatch' | 'revision-conflict' }
  | { state: 'bound'; record: OutboxRecord<TPacket> | undefined };

/**
 * Never even read a differently scoped repository. The unknown result is access feedback,
 * not a replacement record or a second transition engine. No send permission is granted.
 */
export const restoreBoundAttempt = async <TPacket>(
  repository: OutboxRepository<TPacket>, mode: OutboxProfileMode, id: string,
): Promise<BoundAttempt<TPacket>> => {
  if (mode.mode !== 'bound') return { state: 'unknown', reason: 'binding-unavailable' };
  if (!mode.authorized || !sameOutboxScope(repository.scope, mode.scope)) return { state: 'unknown', reason: 'binding-mismatch' };
  const record = await repository.read(id);
  if (!record) return { state: 'bound', record: undefined };
  const restored = restoreOutboxRecord(record);
  if (restored !== record && !await repository.compareAndSwap(record.revision, restored)) {
    return { state: 'unknown', reason: 'revision-conflict' };
  }
  return { state: 'bound', record: restored };
};
