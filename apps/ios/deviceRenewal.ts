import type { PersonPairedSession } from '../../clients/typescript/fractal-client';
import { verifyPairing, verifyPersonRoot, type ProofCrypto } from './pairingProof.ts';
import { isClientApiVersion } from './protocolVersions.ts';

export const renewalResponseValue = (envelope: unknown): unknown => {
  if (!envelope || typeof envelope !== 'object' || !('api_version' in envelope) || !isClientApiVersion(envelope.api_version) || !('value' in envelope)) throw new Error('Invalid renewal response');
  return envelope.value;
};

export type RenewalProfile = { url: string; credential: string; personRootFingerprint?: string; signing?: { key: string; storage: 'secure-enclave' | 'software'; handle?: string; person: string; chain: string[] } };
export type RenewalOffer = {
  renewal_id: string; migration_epoch: string; issuer_member: string; person_id: string; requested_by: string;
  scopes: string[]; old_public_key: string; nonce: string; expires_at_unix_ms: number; grant_expires_at_unix_ms: number;
  person_root_key_proof: unknown; issuer_signature: string;
};
export type RenewalCandidate = { key: string; storage: 'secure-enclave' | 'software'; handle: string };
export type RenewalPorts = {
  crypto: ProofCrypto; now(): number;
  post(url: string, path: string, body: unknown): Promise<unknown>;
  sign(text: string, handle?: string): Promise<string>;
  create(): Promise<RenewalCandidate>;
  approve(text: string, handle: string): Promise<string>;
  remove(handle: string): Promise<void>;
  read(key: string): Promise<RenewalProfile | undefined>;
  save(key: string, profile: RenewalProfile): Promise<void>;
};

/** Subscribe at the store boundary so launch and foreground use the same renewal path. */
export const watchRenewalForeground = (gate: { active: boolean; subscribe(listener: (active: boolean) => void): () => void }, poll: () => Promise<void>): (() => void) => {
  if (gate.active) void poll();
  return gate.subscribe(active => { if (active) void poll(); });
};
const requireValue = (condition: unknown, message: string): void => { if (!condition) throw new Error(message); };
export const pendingRenewalText = (key: string, timestamp: number): string => `fractal-device-renewal-pending-v1\n${JSON.stringify([key, timestamp])}`;
const tuple = (offer: RenewalOffer): unknown[] => [offer.renewal_id, offer.migration_epoch, offer.issuer_member, offer.person_id, offer.requested_by, offer.scopes, offer.old_public_key, offer.nonce, offer.expires_at_unix_ms, offer.grant_expires_at_unix_ms];
export const renewalOfferText = (offer: RenewalOffer): string => `fractal-device-renewal-offer-v1\n${JSON.stringify(tuple(offer))}`;
export const renewalApprovalText = (offer: RenewalOffer, candidate: string): string => `fractal-device-renewal-v1\n${JSON.stringify([...tuple(offer), candidate])}`;

const isRenewalOffer = (value: unknown): value is RenewalOffer => {
  if (!value || typeof value !== 'object') return false;
  return 'renewal_id' in value && typeof value.renewal_id === 'string'
    && 'migration_epoch' in value && typeof value.migration_epoch === 'string'
    && 'issuer_member' in value && typeof value.issuer_member === 'string'
    && 'person_id' in value && typeof value.person_id === 'string'
    && 'requested_by' in value && typeof value.requested_by === 'string'
    && 'old_public_key' in value && typeof value.old_public_key === 'string'
    && 'nonce' in value && typeof value.nonce === 'string'
    && 'issuer_signature' in value && typeof value.issuer_signature === 'string'
    && 'person_root_key_proof' in value
    && 'expires_at_unix_ms' in value && typeof value.expires_at_unix_ms === 'number' && Number.isSafeInteger(value.expires_at_unix_ms)
    && 'grant_expires_at_unix_ms' in value && typeof value.grant_expires_at_unix_ms === 'number' && Number.isSafeInteger(value.grant_expires_at_unix_ms)
    && 'scopes' in value && Array.isArray(value.scopes) && value.scopes.length <= 64 && value.scopes.every(scope => typeof scope === 'string');
};

export async function pendingRenewals(profile: RenewalProfile, url: string, apiVersion: string, ports: RenewalPorts): Promise<RenewalOffer[]> {
  if (!profile.signing || !profile.personRootFingerprint) return [];
  const timestamp = ports.now();
  const signature = await ports.sign(pendingRenewalText(profile.signing.key, timestamp), profile.signing.handle);
  const response = await ports.post(url, '/v1/client/pairings/renewals/pending', { api_version: apiVersion, device_public_key: profile.signing.key, timestamp_unix_ms: timestamp, signature });
  if (!response || typeof response !== 'object' || !('kind' in response) || response.kind !== 'device-renewals'
    || !('renewals' in response) || !Array.isArray(response.renewals) || response.renewals.length > 32
    || !response.renewals.every(isRenewalOffer)) throw new Error('Invalid renewal response');
  const offers = response.renewals;
  for (const offer of offers) {
    requireValue(offer.expires_at_unix_ms > ports.now() && offer.expires_at_unix_ms <= ports.now() + 5 * 60_000
      && offer.grant_expires_at_unix_ms > ports.now() && offer.grant_expires_at_unix_ms <= ports.now() + 30 * 24 * 60 * 60_000
      && offer.old_public_key === profile.signing.key && offer.person_id === profile.signing.person, 'Invalid renewal offer');
    const root = await verifyPersonRoot(offer.person_root_key_proof, offer.person_id, profile.personRootFingerprint, ports.crypto);
    requireValue(await ports.crypto.verify(root, renewalOfferText(offer), offer.issuer_signature), 'Renewal offer signature does not verify');
  }
  return offers;
}

/** The single secure profile write is the commit; failed proofs and writes retain the old profile. */
export async function approveRenewal(profileKey: string, previous: RenewalProfile, url: string | (() => Promise<string>), offer: RenewalOffer, apiVersion: string, ports: RenewalPorts): Promise<RenewalProfile> {
  requireValue(previous.personRootFingerprint && previous.signing && offer.old_public_key === previous.signing.key && offer.person_id === previous.signing.person && offer.expires_at_unix_ms > ports.now(), 'Renewal is no longer available');
  const root = await verifyPersonRoot(offer.person_root_key_proof, offer.person_id, previous.personRootFingerprint!, ports.crypto);
  requireValue(await ports.crypto.verify(root, renewalOfferText(offer), offer.issuer_signature), 'Renewal offer signature does not verify');
  const candidate = await ports.create();
  let committed = false;
  try {
    requireValue(candidate.storage === 'secure-enclave', 'Renewal requires this phone’s Secure Enclave');
    const text = renewalApprovalText(offer, candidate.key);
    const candidateSignature = await ports.approve(text, candidate.handle);
    const oldSignature = await ports.sign(text, previous.signing!.handle);
    const completionUrl = typeof url === 'string' ? url : await url();
    requireValue(offer.expires_at_unix_ms > ports.now(), 'Renewal expired; reopen the app to refresh it');
    const session = await ports.post(completionUrl, '/v1/client/pairings/renewals/complete', { api_version: apiVersion, renewal_id: offer.renewal_id, nonce: offer.nonce, expires_at_unix_ms: offer.expires_at_unix_ms, device_public_key: previous.signing!.key, candidate_public_key: candidate.key, key_storage: candidate.storage, old_signature: oldSignature, candidate_signature: candidateSignature }) as PersonPairedSession;
    requireValue(session.kind === 'paired-session' && session.person_id === offer.person_id && JSON.stringify(session.scopes) === JSON.stringify(offer.scopes), 'Renewal returned a different person or scopes');
    requireValue(Date.parse(session.expires_at) === offer.grant_expires_at_unix_ms, 'Renewal returned a different expiry');
    await verifyPairing(session, candidate.key, previous.personRootFingerprint, ports.crypto);
    requireValue(JSON.stringify(await ports.read(profileKey)) === JSON.stringify(previous), 'The saved pairing changed; renewal was not saved');
    const next: RenewalProfile = { ...previous, credential: session.credential, signing: { key: candidate.key, storage: candidate.storage, handle: candidate.handle, person: session.person_id, chain: session.device_key_chain ?? [] } };
    await ports.save(profileKey, next);
    committed = true;
    if (previous.signing?.handle) await ports.remove(previous.signing.handle).catch(() => {});
    return next;
  } finally { if (!committed) await ports.remove(candidate.handle).catch(() => {}); }
}
