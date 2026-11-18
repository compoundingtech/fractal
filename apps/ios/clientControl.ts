import { CLIENT_RESULT_PURPOSE, type ClientControlGrant, type ClientResultBinding, type ClientVisibleState, type DeviceCommand, type PhoneNavigateArgs } from '../../clients/typescript/fractal-client';

export interface ClientControlPorts {
  identity: { device: string; session: string };
  foreground(): boolean;
  visible(): ClientVisibleState | undefined;
  navigate(args: PhoneNavigateArgs): Promise<void>;
  result(binding: ClientResultBinding, current: () => boolean): Promise<void>;
  revoke(): Promise<void>;
}
const v1Screen = (screen: string): boolean => screen === 'NowRoot' || screen === 'Attention';
export class ClientControlController {
  private grant: ClientControlGrant | undefined;
  private listeners = new Set<() => void>();
  private expires: ReturnType<typeof setTimeout> | undefined;
  private pending = Promise.resolve();
  private disposed = false;
  private revoked = false;
  private epoch = 0;
  private seen = new Map<string, Set<string>>();
  constructor(private readonly ports: ClientControlPorts) {}
  // TODO #6274: only a generated lifecycle resource may publish the idle run indicator.
  readonly getGrant = (): ClientControlGrant | undefined => undefined;
  readonly subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly suspend = (): void => {
    this.epoch++; clearTimeout(this.expires); this.grant = undefined;
    for (const listener of this.listeners) listener();
  };
  readonly dispose = (): void => { this.disposed = true; this.suspend(); this.listeners.clear(); };
  readonly revoke = async (): Promise<void> => { this.revoked = true; this.suspend(); await this.ports.revoke(); };
  private authorized(command: DeviceCommand, grant: ClientControlGrant): boolean {
    const now = Date.now();
    return !this.disposed && !this.revoked && grant.state === 'active'
      && command.device_id === this.ports.identity.device && grant.device === this.ports.identity.device
      && command.session_actor === this.ports.identity.session && grant.session === this.ports.identity.session
      && command.grant_id === grant.grant_id && command.run_id === grant.run_id && command.issued_by === grant.actor_agent
      && Date.parse(grant.expires_at) > now && Date.parse(command.expires_at) > now
      && (command.command_kind === 'phone.inspect' || command.command_kind === 'phone.navigate') && grant.verbs.includes(command.command_kind)
      && (command.command_kind !== 'phone.navigate' || v1Screen(command.args.screen) && grant.screens.includes(command.args.screen)
        && (!grant.item_id || command.args.screen !== 'Attention' || command.args.params.id === grant.item_id));
  }
  readonly handle = (command: DeviceCommand): Promise<void> => {
    const epoch = this.epoch;
    const task = this.pending.then(() => epoch === this.epoch ? this.apply(command) : undefined);
    this.pending = task.catch(() => {});
    return task;
  };
  private async apply(command: DeviceCommand): Promise<void> {
    const grant = command.control_grant;
    if (grant?.grant_id === this.grant?.grant_id && grant?.state !== 'active') this.suspend();
    if (!grant || command.state !== 'delivered' || !this.authorized(command, grant)) return;
    const seen = this.seen.get(grant.grant_id) ?? new Set<string>();
    if (seen.has(command.id) || seen.size >= grant.max_commands) return;
    seen.add(command.id); this.seen.set(grant.grant_id, seen);
    this.grant = grant; clearTimeout(this.expires);
    this.expires = setTimeout(this.suspend, Math.min(Date.parse(grant.expires_at) - Date.now(), 2_147_483_647));
    for (const listener of this.listeners) listener();
    const epoch = this.epoch;
    const current = () => epoch === this.epoch && this.grant === grant && this.authorized(command, grant);
    const foreground = this.ports.foreground();
    if (foreground && command.command_kind === 'phone.navigate') await this.ports.navigate(command.args);
    if (!current()) return;
    const visible = this.ports.visible();
    if (!visible || !v1Screen(visible.screen) || !grant.screens.includes(visible.screen)
      || grant.item_id && visible.screen === 'Attention' && visible.opened_item?.id !== grant.item_id) return;
    await this.ports.result({ purpose: CLIENT_RESULT_PURPOSE, command_kind: command.command_kind === 'phone.inspect' ? 'phone.inspect' : 'phone.navigate',
      device: grant.device, session: grant.session, grant_id: grant.grant_id, run_id: grant.run_id, command_id: command.id,
      outcome: foreground && this.ports.foreground() ? 'completed' : 'not-foreground', error_code: null, visible_state: visible }, current);
  }
}
