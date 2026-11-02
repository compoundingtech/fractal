import { ClientError, isTransient, retryTransient, type Capabilities, type DeviceCommand, type DeviceResult, type FractalClient, type PhoneNavigateArgs, type Snapshot } from '../../clients/typescript/fractal-client';
import type { Foreground } from './terminalControls';

export const phoneCommandsGranted = (caps: Capabilities | null): boolean =>
  !!caps?.capabilities.some(cap => cap.id === 'phone-commands' && cap.version === 1 && cap.state === 'granted');
export type PhoneCheck = (snapshot?: Snapshot) => void;
export class PhoneCommandStopped extends Error {
  constructor(readonly outcome: 'expired' | 'not-foreground') { super(outcome); }
}
export type PhoneBindings = {
  online(): boolean;
  inspect(): unknown;
  navigate(args: PhoneNavigateArgs): Promise<unknown>;
  markRead(id: string, check: PhoneCheck, deviceCommand: string): Promise<boolean>;
};

/** One executor per paired client. The daemon admits and settles commanded writes;
 * backgrounding permanently invalidates work even if awaited responses arrive after resuming. */
export class PhoneCommands {
  private seen = new Map<string, number>();
  private queue = Promise.resolve();
  private generation = 0;
  private closed = false;
  private readonly stop: () => void;
  constructor(private readonly client: Pick<FractalClient, 'deviceResult' | 'capabilities'>, private readonly bindings: () => PhoneBindings, private readonly foreground: Foreground) {
    this.stop = foreground.subscribe(active => { if (!active) this.generation++; });
  }
  close(): void { this.closed = true; this.stop(); }
  receive(commands: DeviceCommand[], snapshot: Snapshot): Promise<void> {
    const now = Date.parse(snapshot.created_at), generation = this.generation;
    for (const [id, expiry] of this.seen) if (expiry <= now) this.seen.delete(id);
    for (const command of commands) {
      const expiry = Date.parse(command.expires_at);
      if (this.closed || !this.foreground.active || !Number.isFinite(expiry) || expiry <= now || this.seen.has(command.id)
        || (command.state !== 'pending' && command.state !== 'delivered')) continue;
      this.seen.set(command.id, expiry);
      this.queue = this.queue.then(() => this.execute(command, generation));
    }
    return this.queue;
  }
  private async execute(command: DeviceCommand, generation: number): Promise<void> {
    const session = () => !this.closed && generation === this.generation && this.foreground.active;
    const check: PhoneCheck = snapshot => {
      if (!session() || !this.bindings().online()) throw new PhoneCommandStopped('not-foreground');
      if (snapshot && Date.parse(snapshot.created_at) >= Date.parse(command.expires_at)) throw new PhoneCommandStopped('expired');
    };
    let report: DeviceResult = { command_id: command.id, outcome: 'completed' };
    try {
      check();
      if (command.command_kind !== 'phone.message.read') check((await this.client.capabilities()).snapshot);
      const bindings = this.bindings();
      switch (command.command_kind) {
        case 'phone.inspect': report.result = bindings.inspect(); break;
        case 'phone.navigate': report.result = await bindings.navigate(command.args); break;
        case 'phone.message.read':
          if (!await bindings.markRead(command.args.message_id, check, command.id)) report = { ...report, outcome: 'refused', error_code: 'message-unavailable' };
          break;
      }
    } catch (error) {
      const refused = error instanceof ClientError && (error.status === 401 || error.status === 403);
      const expired = error instanceof ClientError && error.status === 403 && error.response.details.reason === 'expired';
      report = { ...report, outcome: error instanceof PhoneCommandStopped ? error.outcome : expired ? 'expired' : refused ? 'refused' : 'failed', error_code: error instanceof ClientError ? error.response.code : 'phone-handler-stopped' };
    }
    try {
      await retryTransient(3, async () => { if (session()) await this.client.deviceResult(report); }, error => session() && isTransient(error));
    } catch { /* A lost report never replays the handler. Daemon expiry remains authoritative. */ }
  }
}
