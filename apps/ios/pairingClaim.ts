import { API_VERSION, ClientError } from '../../clients/typescript/fractal-client';
import type { DeviceKeyInfo } from './modules/fractal-device-key';

export type PairingCandidate = { gateway: string; id: string; code: string };
export type PairingClaim = PairingCandidate & {
  key: DeviceKeyInfo;
  sign(text: string, handle: string): Promise<string>;
};

export type ClaimPorts = {
  fetch: typeof fetch;
  hash(bytes: Uint8Array): Promise<Uint8Array>;
};

/** PR #6291: three UTF-8 lines, lowercase code digest, no trailing LF; native sign returns raw r||s. */
export async function pairingClaimRequest(claim: PairingClaim, hash: ClaimPorts['hash']) {
  const digest = await hash(new TextEncoder().encode(claim.code));
  const hex = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
  const proof = await claim.sign(`pairing.claim.v1\n${claim.id}\n${hex}`, claim.key.handle);
  return { api_version: API_VERSION, code: claim.code, device_public_key: claim.key.key, proof };
}

/** Wire fields, proof encoding and server refusals belong at this one boundary. */
export async function claimPairingKey(claim: PairingClaim, ports: ClaimPorts): Promise<void> {
  const response = await ports.fetch(`${claim.gateway}/v1/client/capabilities?accepts=pairing-key-binding`);
  const text = await response.text();
  if (!response.ok || text.length > 8192) throw new Error('Pairing refused: scanning-key capability is unavailable.');
  const advertisement = JSON.parse(text);
  if (advertisement.api_version !== API_VERSION || !Array.isArray(advertisement.capabilities)
    || !advertisement.capabilities.some((capability: { id?: string; version?: number; state?: string }) =>
      capability?.id === 'pairing-key-binding' && capability.version === 2 && capability.state === 'granted')) {
    throw new Error('Pairing refused: this gateway does not support scanning-key claims.');
  }
  await pairingClaimRequest(claim, ports.hash);
  // The generated SDK binding for POST /v1/client/pairings/{id}/claim is still pending on PR #6291.
  // Never submit via an untyped parallel client, nor fall back to unbound completion.
  throw new Error('Pairing refused: this test build cannot submit a scanning-key claim yet. Update its generated SDK.');
}

/** Preserve typed server refusals, including pairing-key-binding-required, without retrying. */
export function pairingClaimRefusal(error: unknown): string | undefined {
  if (error instanceof ClientError) return `Pairing refused (${error.response.code}). Start a fresh pairing on the trusted machine.`;
  if (error instanceof Error && error.message.startsWith('Pairing refused:')) return error.message;
  return undefined;
}
