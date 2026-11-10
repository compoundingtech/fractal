import { normalizeGatewayUrl } from './gatewayUrl';

export type PairingQrResult =
  | { ok: true; gateway: string; id: string; code: string; expiresAtUnixMs: number }
  | { ok: false; reason: 'malformed' | 'expired' | 'gateway' };

/** Decode only: scanning never pairs, changes a draft, or trusts a root fingerprint. */
export const parsePairingQr = (raw: string, now = Date.now()): PairingQrResult => {
  if (raw.length > 4096 || raw !== raw.trim()) return { ok: false, reason: 'malformed' };
  let value: unknown;
  try { value = JSON.parse(raw); } catch { return { ok: false, reason: 'malformed' }; }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return { ok: false, reason: 'malformed' };
  const fields = Object.keys(value);
  if (fields.length !== 6 || !fields.every(key => ['_tag', 'version', 'gateway', 'id', 'code', 'expires_at_unix_ms'].includes(key))) {
    // Six exact fields: reject additions, including any claimed fingerprint.
    return { ok: false, reason: 'malformed' };
  }
  if (!('_tag' in value) || value._tag !== 'fractal.pairing' || !('version' in value) || value.version !== 1
    || !('gateway' in value) || typeof value.gateway !== 'string' || value.gateway.length > 2048
    || !('id' in value) || typeof value.id !== 'string' || !/^pairing\/[a-f0-9]{24}$/.test(value.id)
    || !('code' in value) || typeof value.code !== 'string' || !/^[ABCDEFGHJKLMNPQRSTUVWXYZ23456789]{8}$/.test(value.code)
    || !('expires_at_unix_ms' in value) || typeof value.expires_at_unix_ms !== 'number'
    || !Number.isSafeInteger(value.expires_at_unix_ms) || value.expires_at_unix_ms <= 0) return { ok: false, reason: 'malformed' };
  const gateway = normalizeGatewayUrl(value.gateway);
  if (!gateway) return { ok: false, reason: 'gateway' };
  if (value.gateway.includes('?') || value.gateway.includes('#') || value.gateway !== value.gateway.trim()) return { ok: false, reason: 'gateway' };
  if (value.expires_at_unix_ms <= now) return { ok: false, reason: 'expired' };
  return { ok: true, gateway, id: value.id, code: value.code, expiresAtUnixMs: value.expires_at_unix_ms };
};

