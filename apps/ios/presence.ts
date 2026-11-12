// This phone tells Fractal whether the person is here: present while the app is in the foreground,
// idle once it leaves. iOS gives an app no reliable lock signal, and `inactive` is transient (a Share
// sheet, Control Center), so neither is reported. Fractal counts a report for 15 minutes and records
// an unchanged one at most every 5, so while the app stays in front it repeats present every 10.
import { ClientError } from '../../clients/typescript/fractal-client';

export const PRESENT_EVERY_MS = 10 * 60_000;

export type PresencePorts = {
  report(presence: 'present' | 'idle', observedAt: string): Promise<unknown>;
  /** Run `run` once after `ms`; the answer cancels it. */
  after(run: () => void, ms: number): () => void;
  now(): number;
};

export class PresenceReporter {
  private cancel: (() => void) | null = null;
  private refused = false;

  constructor(private readonly ports: PresencePorts) {}

  /** The app is in the foreground (or a connection went live while it is), or it left. */
  foreground(active: boolean): void {
    this.stop();
    if (this.refused) return;
    void this.send(active ? 'present' : 'idle');
    if (active) this.cancel = this.ports.after(() => this.foreground(true), PRESENT_EVERY_MS);
  }

  stop(): void {
    this.cancel?.();
    this.cancel = null;
  }

  // A refusal (a pairing without `write.presence`) ends reporting for this connection; any other
  // failure leaves the next report to try again.
  private async send(presence: 'present' | 'idle'): Promise<void> {
    try { await this.ports.report(presence, new Date(this.ports.now()).toISOString()); } catch (error) {
      if (error instanceof ClientError && error.status === 403) { this.refused = true; this.stop(); }
    }
  }
}
