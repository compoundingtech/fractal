import type { CarrierChoice } from './carrier';
import type { FabricMember } from './fabricTarget';

export type PairingRouting = { carrier: CarrierChoice; members: FabricMember[] };

/** A scanned code is never an authority for the person's root identity. */
export const pairingTrustAllowed = (scanned: boolean, unpinned: boolean): boolean => !scanned || !unpinned;

/** A QR authenticates a gateway, not any Fabric node. Rebind routing with the bearer in one write. */
export const pairingCommitRouting = (previous: PairingRouting, scanned: boolean): PairingRouting => {
  if (!scanned) return previous;
  return {
    carrier: 'tailscale',
    members: previous.members.map(({ device: _device, ...member }) => member),
  };
};
