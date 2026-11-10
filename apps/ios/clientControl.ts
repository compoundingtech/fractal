import type { DeviceCommand, PhoneNavigateArgs } from '../../clients/typescript/fractal-client';
import { authorizeClientControl, revokeClientControl } from './clientControlWire.ts';

/** App-local committed route; item IDs are present only when actually observed. */
export type VisibleState =
  | { readonly screen: 'NowRoot'; readonly itemIds?: readonly string[] }
  | { readonly screen: 'Attention'; readonly id: string; readonly itemIds?: readonly string[] };

/** UI projection only. This is not a wire grant and cannot establish authority. */
export interface ActiveControlGrant {
  readonly runId: string;
  readonly revokeState: 'available' | 'pending' | 'failed';
}

export interface ClientControlPorts {
  navigate(args: PhoneNavigateArgs): void;
  visible(): VisibleState | undefined;
  foreground(): boolean;
}

/** Pure navigation only: no answer, close, read-state or result capability. */
export const navigateClientControl = (args: PhoneNavigateArgs, navigate: ClientControlPorts['navigate']): boolean => {
  if (args.screen !== 'NowRoot' && args.screen !== 'Attention') return false;
  navigate(args);
  return true;
};

/** Local policy only; the wire adapter must independently establish grant authority. */
export const applyGrantedClientControl = (command: DeviceCommand, scope: { verbs: readonly string[]; screens: readonly string[] }, ports: ClientControlPorts): void => {
  if (!scope.verbs.includes(command.command_kind) || !ports.foreground()) return;
  if (command.command_kind === 'phone.inspect') { ports.visible(); return; }
  if (command.command_kind !== 'phone.navigate' || !scope.screens.includes(command.args.screen)) return;
  navigateClientControl(command.args, ports.navigate);
};

export class ClientControlController {
  constructor(private readonly ports: ClientControlPorts) {}

  // #6274: no authoritative generated grant source exists. The snapshot is
  // permanently absent, so there are no store changes to subscribe to yet.
  readonly getGrant = (): ActiveControlGrant | undefined => undefined;
  readonly subscribe = (_listener: () => void): (() => void) => () => {};

  readonly handle = async (command: DeviceCommand): Promise<void> => {
    const grant = this.getGrant();
    if (!authorizeClientControl(command, { grant, foreground: this.ports.foreground })) return;
    if (!grant) return;
    applyGrantedClientControl(command, { verbs: ['phone.inspect', 'phone.navigate'], screens: ['NowRoot', 'Attention'] }, this.ports);
    // No result is posted: signing/encoding must come from #6274 codegen.
  };

  readonly revoke = async (): Promise<void> => {
    // Do not clear UI state or claim enrollment ended when wire revoke is absent.
    await revokeClientControl(this.getGrant());
  };
}
