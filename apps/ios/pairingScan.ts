import { normalizeGatewayUrl } from './gatewayUrl';
import type { PairingQrResult } from './pairingQr';

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
