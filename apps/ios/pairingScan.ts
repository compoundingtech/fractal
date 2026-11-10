import { normalizeGatewayUrl } from './gatewayUrl';
import type { PairingQrResult } from './pairingQr';
import type { DeviceKeyInfo } from './modules/fractal-device-key';
import type { PairingCandidate, PairingClaim } from './pairingClaim';

export const isTestPairingClient = (extra: { testClientMode?: unknown } | undefined): boolean => extra?.testClientMode === true;
export type PairingScanSession = {
  discard(): Promise<void>;
  scan(camera: () => Promise<string | null>): Promise<string | null>;
  claim(candidate: PairingCandidate): Promise<void>;
  takeCompletionKey(candidate: PairingCandidate): DeviceKeyInfo;
};

/** The store owns one scanning key; the camera, claim and completion share this lifecycle. */
export function pairingScanSession(testClient: boolean, ports: {
  create(): Promise<DeviceKeyInfo | null>;
  remove(handle: string): Promise<void>;
  sign(text: string, handle: string): Promise<string>;
  claim(claim: PairingClaim): Promise<void>;
}): PairingScanSession {
  let prepared: DeviceKeyInfo | undefined;
  let claimed: PairingCandidate | undefined;
  const discard = async () => {
    const old = prepared;
    prepared = undefined; claimed = undefined;
    if (old) await ports.remove(old.handle).catch(() => {});
  };
  return {
    discard,
    async scan(camera: () => Promise<string | null>): Promise<string | null> {
      if (!testClient) return camera();
      await discard();
      prepared = await ports.create() ?? undefined;
      if (!prepared) throw new Error('Pairing refused: this build cannot make a device key.');
      try {
        const raw = await camera();
        if (raw === null) await discard();
        return raw;
      } catch (error) { await discard(); throw error; }
    },
    async claim(candidate: PairingCandidate): Promise<void> {
      if (!testClient) return;
      if (!prepared || claimed) throw new Error('Pairing refused: scan a fresh QR before claiming a key.');
      try {
        await ports.claim({ ...candidate, key: prepared, sign: ports.sign });
        claimed = candidate;
      } catch (error) { await discard(); throw error; }
    },
    takeCompletionKey(candidate: PairingCandidate): DeviceKeyInfo {
      if (!prepared || !claimed || candidate.gateway !== claimed.gateway || candidate.id !== claimed.id || candidate.code !== claimed.code) {
        throw new Error('Pairing refused: scan and claim this pairing with this device first.');
      }
      const key = prepared;
      prepared = undefined; claimed = undefined;
      return key;
    },
  };
}

export const CAMERA_DENIED = 'Camera access is off. Allow it in iOS Settings, or enter the pairing details below.';
export const SCAN_FAILED = 'Could not scan a pairing QR code. Try again, or enter the details by hand.';
const issues = {
  malformed: 'This QR is not a Fractal pairing code. Scan the QR shown by fractal devices pair, or enter the details by hand.',
  expired: 'This pairing QR expired. Start a new pairing on the trusted machine and scan the new QR.',
  gateway: 'This QR needs a supported gateway: HTTPS, or HTTP on Tailscale or a trusted local network.',
} as const;

type Candidate = { gateway: string; id: string; code: string };
export type ScanOutcome =
  | { kind: 'unchanged' }
  | { kind: 'issue'; text: string }
  | { kind: 'prefill'; candidate: Candidate }
  | { kind: 'confirm-gateway'; candidate: Candidate };

/** No saved profile or fingerprint participates in a QR candidate. Cancel does not change a draft. */
export const scanOutcome = (scanned: PairingQrResult | null, currentGateway: string): ScanOutcome => {
  if (scanned === null) return { kind: 'unchanged' };
  if (!scanned.ok) return { kind: 'issue', text: issues[scanned.reason] };
  const candidate = { gateway: scanned.gateway, id: scanned.id, code: scanned.code };
  return !currentGateway || normalizeGatewayUrl(currentGateway) === scanned.gateway
    ? { kind: 'prefill', candidate }
    : { kind: 'confirm-gateway', candidate };
};
