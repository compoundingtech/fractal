// This phone tells Fractal its raw state: `active` while the app is in the foreground, `inactive`
// once it leaves (iOS gives an app no lock signal, so it never reports `locked` or `sleeping`), each
// with the person's last input. Fractal derives presence from it; the phone applies no idle threshold
// and sends no heartbeat. It reports on every change of state, and again at the answer's
// `report_again_at` (the moment the phone would turn idle) with its newer last input. A null
// `report_again_at` leaves the next report to the next change of state or the first input after it.
import { ClientError, type PresenceReport, type PresenceReportResult } from '../../clients/typescript/fractal-client';

export type PresencePorts = {
  report(report: PresenceReport): Promise<PresenceReportResult>;
  /** When the person last touched the app or brought it forward, as epoch milliseconds. */
  lastInput(): number | null;
  /** Run `run` once after `ms`; the answer cancels it. */
  after(run: () => void, ms: number): () => void;
  now(): number;
};

export class PresenceReporter {
  private state: 'active' | 'inactive' | null = null;
  private cancel: (() => void) | null = null;
  /** The last answer named no time: the next input reports. */
  private awaitingInput = false;
  /** Only the newest report's answer schedules the next one. */
  private sent = 0;
  private ended = false;

  constructor(private readonly ports: PresencePorts) {}

  /** The app came to the foreground (or a connection went live while it is there), or it left. */
  foreground(active: boolean): void {
    const state = active ? 'active' : 'inactive';
    if (this.ended || state === this.state) return;
    this.state = state;
    this.send();
  }

  /** The person touched the app. Only the first touch after an answer without `report_again_at` reports. */
  input(): void {
    if (!this.ended && this.awaitingInput && this.state === 'active') this.send();
  }

  /** The connection ended: nothing more is sent and no answer schedules a report. */
  stop(): void {
    this.ended = true;
    this.clear();
  }

  private clear(): void {
    this.cancel?.();
    this.cancel = null;
    this.awaitingInput = false;
  }

  // A refusal (a pairing without `write.presence`, or a session that is not a person) ends reporting
  // for this connection. Any other failure leaves the next change of state or input to report.
  private send(): void {
    if (this.ended || !this.state) return;
    this.clear();
    const sent = ++this.sent;
    const now = this.ports.now(), input = this.ports.lastInput();
    const report: PresenceReport = { state: this.state, observed_at: new Date(now).toISOString(), ...(input === null ? {} : { last_input_at: new Date(Math.min(input, now)).toISOString() }) };
    this.ports.report(report).then(
      ({ report_again_at: again }) => {
        if (this.ended || sent !== this.sent) return;
        if (again === null) this.awaitingInput = true;
        else this.cancel = this.ports.after(() => { this.cancel = null; this.send(); }, Math.max(0, Date.parse(again) - this.ports.now()));
      },
      (error: unknown) => {
        if (error instanceof ClientError && error.status === 403) this.stop();
        else if (!this.ended && sent === this.sent) this.awaitingInput = true;
      },
    );
  }
}
