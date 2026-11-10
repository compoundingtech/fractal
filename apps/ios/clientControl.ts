import { CLIENT_RESULT_PURPOSE, type ClientControlGrant, type ClientControlGrantLifecycle, type ClientResultBinding, type ClientVisibleState, type DeviceCommand, type PhoneNavigateArgs } from '../../clients/typescript/fractal-client';

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
  private grant: ClientControlGrantLifecycle | undefined;
  private lifecycles = new Map<string, ClientControlGrantLifecycle>();
  private listeners = new Set<() => void>();
  private expires: ReturnType<typeof setTimeout> | undefined;
  private pending = Promise.resolve();
  private disposed = false;
  private revoked = false;
  private epoch = 0;
  private seen = new Map<string, Set<string>>();
  constructor(private readonly ports: ClientControlPorts) {}
  readonly getGrant = (): ClientControlGrantLifecycle | undefined => this.grant;
  readonly subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly suspend = (): void => {
    this.epoch++; clearTimeout(this.expires); this.grant = undefined;
    for (const listener of this.listeners) listener();
  };
  readonly dispose = (): void => { this.disposed = true; this.suspend(); this.listeners.clear(); };
  readonly revoke = async (): Promise<void> => { this.revoked = true; this.suspend(); await this.ports.revoke(); };
  private lifecycle(row: ClientControlGrantLifecycle, snapshot: boolean): void {
    if (this.disposed || this.revoked || row.device !== this.ports.identity.device || row.session !== this.ports.identity.session) return;
    if (!Number.isSafeInteger(row.sequence) || row.sequence <= 0
      || !['started', 'ended', 'revoked'].includes(row.state)) { this.suspend(); return; }
    const previous = this.lifecycles.get(row.grant_id);
    if (previous && (row.run_id !== previous.run_id || row.item_id !== previous.item_id
      || row.sequence < previous.sequence || previous.state !== 'started'
      || row.sequence === previous.sequence && (!snapshot || row.state !== 'started'))) return;
    this.lifecycles.set(row.grant_id, row);
    if (row.state === 'started') {
      this.suspend(); this.grant = row;
      for (const listener of this.listeners) listener();
    } else if (this.grant?.grant_id === row.grant_id) this.suspend();
  }
  private authorized(command: DeviceCommand, grant: ClientControlGrant): boolean {
    const now = Date.now();
    const lifecycle = this.grant;
    return !!lifecycle && lifecycle.state === 'started' && lifecycle.grant_id === grant.grant_id
      && lifecycle.device === grant.device && lifecycle.session === grant.session
      && lifecycle.run_id === grant.run_id && lifecycle.item_id === grant.item_id
      && !this.disposed && !this.revoked && grant.state === 'active'
      && command.device_id === this.ports.identity.device && grant.device === this.ports.identity.device
      && command.session_actor === this.ports.identity.session && grant.session === this.ports.identity.session
      && command.grant_id === grant.grant_id && command.run_id === grant.run_id && command.issued_by === grant.actor_agent
      && Date.parse(grant.expires_at) > now && Date.parse(command.expires_at) > now
      && (command.command_kind === 'phone.inspect' || command.command_kind === 'phone.navigate') && grant.verbs.includes(command.command_kind)
      && (command.command_kind !== 'phone.navigate' || v1Screen(command.args.screen) && grant.screens.includes(command.args.screen)
        && (!grant.item_id || command.args.screen !== 'Attention' || command.args.params.id === grant.item_id));
  }
  readonly handle = (command: DeviceCommand | ClientControlGrantLifecycle, snapshot = false): Promise<void> => {
    if (command.kind === 'client-control-grant-lifecycle') { this.lifecycle(command, snapshot); return Promise.resolve(); }
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
    clearTimeout(this.expires);
    this.expires = setTimeout(this.suspend, Math.min(Date.parse(grant.expires_at) - Date.now(), 2_147_483_647));
    const epoch = this.epoch;
    const current = () => epoch === this.epoch && this.authorized(command, grant);
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
