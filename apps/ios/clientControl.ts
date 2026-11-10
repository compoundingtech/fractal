import type { DeviceCommand, PhoneNavigateArgs } from '../../clients/typescript/fractal-client';
import { authorizeClientControl, revokeClientControl } from './clientControlWire.ts';

/** App-local committed route; item IDs are present only when actually observed. */
export type VisibleState =
  | { readonly screen: 'NowRoot'; readonly itemIds?: readonly string[] }
  | { readonly screen: 'Conversation'; readonly target: string; readonly sessionId?: string; readonly itemIds?: readonly string[] };

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

export class ClientControlController {
  constructor(private readonly ports: ClientControlPorts) {}

  // #6274: no authoritative generated grant source exists. The snapshot is
  // permanently absent, so there are no store changes to subscribe to yet.
  readonly getGrant = (): ActiveControlGrant | undefined => undefined;
  readonly subscribe = (_listener: () => void): (() => void) => () => {};

  readonly handle = async (command: DeviceCommand): Promise<void> => {
    const grant = this.getGrant();
    if (!authorizeClientControl(command, { grant, foreground: this.ports.foreground })) return;
    if (!grant || !this.ports.foreground()) return;
    switch (command.command_kind) {
      case 'phone.inspect':
        this.ports.visible();
        break;
      case 'phone.navigate':
        if (command.args.screen !== 'NowRoot' && command.args.screen !== 'Conversation') return;
        this.ports.navigate(command.args);
        break;
      case 'phone.message.read':
        // Message bodies are outside this controller's visible-route ports.
        break;
    }
    // No result is posted: signing/encoding must come from #6274 codegen.
  };

  readonly revoke = async (): Promise<void> => {
    // Do not clear UI state or claim enrollment ended when wire revoke is absent.
    await revokeClientControl(this.getGrant());
  };
}
