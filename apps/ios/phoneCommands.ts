import { ClientError, isTransient, retryTransient, type Capabilities, type DeviceCommand, type DeviceResult, type FractalClient, type PhoneNavigateArgs, type Snapshot } from '../../clients/typescript/fractal-client';

export const phoneCommandsGranted = (caps: Capabilities | null): boolean =>
  !!caps?.capabilities.some(cap => cap.id === 'phone-commands' && cap.version === 1 && cap.state === 'granted');
export type PhoneBindings = {
  foreground(): boolean;
  online(): boolean;
  inspect(): unknown;
  navigate(args: PhoneNavigateArgs): Promise<unknown>;
  markRead(id: string): Promise<boolean>;
};

/** One executor per paired client, surviving socket reconnects. Deadlines use daemon time plus
 * monotonic elapsed time, never the phone's wall clock. Only result POSTs may retry. */
export class PhoneCommands {
  private seen = new Map<string, number>();
  private queue = Promise.resolve();
  private closed = false;
  constructor(private readonly client: Pick<FractalClient, 'deviceResult'>, private readonly bindings: () => PhoneBindings) {}
  close(): void { this.closed = true; }
  receive(commands: DeviceCommand[], snapshot: Snapshot, received = performance.now()): Promise<void> {
    for (const [id, deadline] of this.seen) if (deadline <= performance.now()) this.seen.delete(id);
    for (const command of commands) {
      const deadline = received + Date.parse(command.expires_at) - Date.parse(snapshot.created_at);
      if (this.closed || !Number.isFinite(deadline) || deadline <= performance.now() || this.seen.has(command.id)
        || (command.state !== 'pending' && command.state !== 'delivered')) continue;
      this.seen.set(command.id, deadline);
      this.queue = this.queue.then(() => this.execute(command, deadline));
    }
    return this.queue;
  }
  private async execute(command: DeviceCommand, deadline: number): Promise<void> {
    const available = () => !this.closed && performance.now() < deadline;
    if (!available()) return;
    const bindings = this.bindings();
    let report: DeviceResult = { command_id: command.id, outcome: 'completed' };
    try {
      if (!bindings.foreground() || !bindings.online()) report.outcome = 'not-foreground';
      else switch (command.command_kind) {
        case 'phone.inspect': report.result = bindings.inspect(); break;
        case 'phone.navigate': report.result = await bindings.navigate(command.args); break;
        case 'phone.message.read':
          if (!await bindings.markRead(command.args.attention_id)) report = { ...report, outcome: 'refused', error_code: 'attention-unavailable' };
          break;
      }
    } catch (error) {
      const refused = error instanceof ClientError && (error.status === 401 || error.status === 403);
      report = { ...report, outcome: refused ? 'refused' : 'failed', error_code: error instanceof ClientError ? error.response.code : 'phone-handler-failed' };
    }
    // Backgrounding/retiring a client cancels reporting; the daemon decides the terminal deadline.
    try {
      await retryTransient(3, async () => {
        if (!available() || !this.bindings().foreground()) return;
        await this.client.deviceResult(report);
      }, error => available() && this.bindings().foreground() && isTransient(error));
    } catch { /* A lost report must never replay the handler. Daemon expiry remains authoritative. */ }
  }
}
