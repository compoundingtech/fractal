import type { DeviceCommand } from '../../clients/typescript/fractal-client';
import type { ActiveControlGrant } from './clientControl.ts';

/** Internal authorization inputs, not an alternate wire grant or signature schema. */
export interface ControlAuthorizationContext {
  readonly grant: ActiveControlGrant | undefined;
  readonly foreground: () => boolean;
}

/**
 * #6274 must generate the authoritative grant, lifecycle, signature and result
 * contracts before this boundary can authorize anything. Existing DeviceCommand
 * rows alone are not proof of a grant. Do not infer authority from their fields.
 */
export const authorizeClientControl = (
  _command: DeviceCommand,
  _context: ControlAuthorizationContext,
): boolean => false;

/** No wire revoke exists yet; false must never be presented as a successful revoke. */
export const revokeClientControl = async (_grant: ActiveControlGrant | undefined): Promise<boolean> => false;
