import type { PushRegistration, PushRegistrationStatus } from '../../clients/typescript/st3-client';

export type PushEnvironment = PushRegistration['environment'];
export type PushDecision = 'none' | 'register' | 'unregister';
export type PushEligibility = { paired: boolean; granted: boolean; enabled: boolean; available: boolean; permission: boolean };

/** Disabling (or losing permission) removes the member's registration, even on a non-push build. */
export const pushDecision = (state: PushEligibility): PushDecision => {
  if (!state.paired || !state.granted) return 'none';
  if (!state.enabled || !state.permission) return 'unregister';
  return state.available ? 'register' : 'none';
};

/** Native input is never echoed into errors. Tokens stay in the request, not app state/storage. */
export const pushRequest = (token: unknown, environment: PushEnvironment, bundleId: string | null, version: string | null): PushRegistration | undefined => {
  if (typeof token !== 'string' || !/^(?:[a-fA-F0-9]{2}){1,256}$/.test(token)) return;
  if (!bundleId || !/^[A-Za-z0-9.-]{1,255}$/.test(bundleId)) return;
  if (!version || !/^[\x20-\x7e]{1,64}$/.test(version)) return;
  return { token, environment, bundle_id: bundleId, app_version: version };
};

export type PushRemote = {
  register(request: PushRegistration): Promise<PushRegistrationStatus>;
  unregister(): Promise<PushRegistrationStatus>;
};
export type PushContext = { paired: boolean; granted: boolean; remote?: PushRemote };
export type PushState = {
  enabled: boolean;
  loaded: boolean;
  environment?: PushEnvironment;
  status: 'idle' | 'working' | 'registered' | 'unregistered' | 'permission-denied' | 'failed';
};
export type PushPorts = {
  readEnabled(): Promise<boolean>;
  writeEnabled(enabled: boolean): Promise<void>;
  environment(): Promise<PushEnvironment | undefined>;
  permission(request: boolean): Promise<boolean>;
  token(): Promise<unknown>;
  listen(changed: (token: unknown) => void): () => void;
  bundleId: string | null;
  version: string | null;
};

/** One phone-local lifecycle. Serialize writes so opt-out follows an already-sent register.
 * Native acquisition is generation-fenced; failed transport writes reconcile on the next connection.
 */
export class PushRegistrationLifecycle {
  private state: PushState = { enabled: false, loaded: false, status: 'idle' };
  private context: PushContext = { paired: false, granted: false };
  private listeners = new Set<() => void>();
  private lastRemote?: PushRemote;
  private stopTokens?: () => void;
  private loading?: Promise<void>;
  private queue: Promise<void> = Promise.resolve();
  private generation = 0;
  private nativeWaiters = new Set<() => void>();
  private forgetting = false;
  private disposed = false;

  constructor(private readonly ports: PushPorts) {}

  private invalidate(): void {
    this.generation++;
    for (const cancel of this.nativeWaiters) cancel();
    this.nativeWaiters.clear();
  }
  /** Native APNs acquisition can wait indefinitely. Opt-out/forget must not wait for that callback. */
  private async readNative<T>(read: () => Promise<T>, generation: number): Promise<T | undefined> {
    if (generation !== this.generation) return;
    let cancel = () => {};
    const canceled = new Promise<undefined>(resolve => { cancel = () => resolve(undefined); });
    this.nativeWaiters.add(cancel);
    try { return await Promise.race([read(), canceled]); }
    finally { this.nativeWaiters.delete(cancel); }
  }

  readonly snapshot = (): PushState => this.state;
  readonly subscribe = (changed: () => void): (() => void) => {
    this.listeners.add(changed);
    return () => { this.listeners.delete(changed); };
  };
  private publish(patch: Partial<PushState>): void {
    this.state = { ...this.state, ...patch };
    for (const changed of this.listeners) changed();
  }
  private enqueue(job: () => Promise<void>): Promise<void> {
    this.queue = this.queue.then(job).catch(() => {
      if (!this.disposed && !this.forgetting) this.publish({ status: 'failed' });
    });
    return this.queue;
  }
  private eligible(): boolean {
    return !this.disposed && !this.forgetting && this.state.loaded && this.context.paired && this.context.granted && !!this.context.remote;
  }
  private listen(): void {
    this.stopTokens?.(); this.stopTokens = undefined;
    if (!this.eligible() || !this.state.enabled || !this.state.environment) return;
    const generation = this.generation;
    this.stopTokens = this.ports.listen(token => {
      if (generation === this.generation) void this.enqueue(() => this.reconcile(false, token));
    });
  }
  start(): Promise<void> {
    this.disposed = false;
    if (!this.loading) this.loading = (async () => {
      const [enabled, environment] = await Promise.all([this.ports.readEnabled(), this.ports.environment()]);
      this.publish({ enabled, environment, loaded: true });
    })().catch(() => { this.publish({ loaded: true, status: 'failed' }); });
    return this.loading.then(() => {
      if (this.disposed) return;
      this.listen();
      return this.refresh();
    });
  }
  configure(context: PushContext): void {
    this.invalidate();
    if (context.paired && context.granted && context.remote) this.lastRemote = context.remote;
    if (!context.paired) this.lastRemote = undefined;
    this.context = context;
    this.listen();
    if (this.state.loaded) void this.refresh();
  }
  stop(): void {
    this.disposed = true; this.invalidate();
    this.stopTokens?.(); this.stopTokens = undefined;
  }
  refresh(): Promise<void> { return this.enqueue(() => this.reconcile(false)); }

  /** Only this intentional action can request iOS permission. Enabling cannot happen unpaired. */
  setEnabled(enabled: boolean): Promise<void> {
    if (enabled && (!this.eligible() || !this.state.environment)) return Promise.resolve();
    this.invalidate();
    this.publish({ enabled, status: 'working' });
    this.listen();
    const generation = this.generation;
    return this.enqueue(async () => {
      await this.ports.writeEnabled(enabled);
      if (generation !== this.generation) return;
      await this.reconcile(enabled);
    });
  }
  private async reconcile(requestPermission: boolean, changedToken?: unknown): Promise<void> {
    const remote = this.context.remote;
    if (!this.eligible() || !remote) return;
    const generation = this.generation;
    const current = () => this.eligible() && generation === this.generation;
    this.publish({ status: 'working' });
    const permission = this.state.enabled && this.state.environment ? await this.readNative(() => this.ports.permission(requestPermission), generation) : false;
    if (!current()) return;
    const decision = pushDecision({ paired: this.context.paired, granted: this.context.granted, enabled: this.state.enabled, available: !!this.state.environment, permission: permission === true });
    if (decision === 'unregister') {
      await remote.unregister();
      if (current()) this.publish({ status: this.state.enabled ? 'permission-denied' : 'unregistered' });
      return;
    }
    if (decision !== 'register' || !this.state.environment) return;
    const token = changedToken ?? await this.readNative(() => this.ports.token(), generation);
    if (!current()) return;
    const request = pushRequest(token, this.state.environment, this.ports.bundleId, this.ports.version);
    if (!request) { this.publish({ status: 'failed' }); return; }
    const result = await remote.register(request);
    if (current()) this.publish({ status: result.registered ? 'registered' : 'unregistered' });
  }

  /** Fence native work immediately, then unregister after any already-sent registration settles. */
  forget(): Promise<void> {
    const remote = this.lastRemote;
    this.forgetting = true; this.invalidate();
    this.stopTokens?.(); this.stopTokens = undefined;
    return this.enqueue(async () => {
      await this.ports.writeEnabled(false).catch(() => {});
      if (remote) await remote.unregister().catch(() => {});
      this.publish({ enabled: false, status: 'unregistered' });
    });
  }
  resume(): void { this.forgetting = false; }
}
