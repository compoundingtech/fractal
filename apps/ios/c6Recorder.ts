// C6 is opt-in, device-local, and never part of observation reporting.
export type C6Context = { route: 'tailscale' | 'lan' | 'https' | 'fabric' | 'none'; path: 'direct' | 'relay' | 'unknown' };
export type C6Row = {
  runId: string; blockId: string; index: number; operation: 'status' | 'send-echo';
  version: string; start: C6Context; end: C6Context; requestId: string | null; messageId: string | null;
  ackMs: number | null; echoMs: number | null; status: 'ok' | 'failed' | 'timeout' | 'invalid';
  errorCode: 'request-failed' | 'timeout' | 'route-changed' | null;
};
export type C6Send = { signal: AbortSignal; request: (id: string) => void; ack: () => void };
export const C6_LIMIT = 200;
export const C6_TIMEOUT_MS = 15_000;
export const nearestRank = (values: readonly number[], percentile: number): number | null => {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.min(sorted.length - 1, Math.ceil(percentile * sorted.length) - 1))];
};
export class C6Recorder {
  rows: readonly C6Row[] = [];
  busy = false;
  private pending?: { nonce: string; observed: (id: string) => void };
  constructor(private ports: { now: () => number; id: () => string; context: () => C6Context; version: string; changed: () => void; timeoutMs?: number }) {}
  clear(): void { if (!this.busy) { this.rows = []; this.ports.changed(); } }
  observe(id: string, text: string, own: boolean): void {
    const pending = this.pending;
    if (own && pending && text.includes(`[C6:${pending.nonce}]`)) pending.observed(id);
  }
  export(): string { return JSON.stringify({ schema: 'ios-c6.v1', rows: this.rows }, null, 2); }
  async run(operation: C6Row['operation'], execute: (nonce: string, hooks: C6Send) => Promise<void>): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.ports.changed();
    const runId = this.ports.id(), blockId = this.ports.id();
    try {
      for (let index = 1; index <= 10; index++) {
        const start = { ...this.ports.context() }, began = this.ports.now();
        const row: C6Row = { runId, blockId, index, operation, version: this.ports.version, start, end: start, requestId: null, messageId: null, ackMs: null, echoMs: null, status: 'ok', errorCode: null };
        const controller = new AbortController(), nonce = this.ports.id();
        let timer: ReturnType<typeof setTimeout> | undefined;
        let active = true;
        const echo = new Promise<void>(resolve => {
          this.pending = { nonce, observed: id => { if (active && row.echoMs === null) { row.messageId = id; row.echoMs = this.ports.now() - began; resolve(); } } };
        });
        try {
          await Promise.race([
            (async () => {
              await execute(nonce, {
                signal: controller.signal,
                request: id => { if (active) row.requestId = id; },
                ack: () => { if (active && row.ackMs === null) row.ackMs = this.ports.now() - began; },
              });
              if (operation === 'send-echo') await echo;
            })(),
            new Promise<never>((_, reject) => { timer = setTimeout(() => { controller.abort(); reject(new Error('timeout')); }, this.ports.timeoutMs ?? C6_TIMEOUT_MS); }),
          ]);
        } catch {
          row.status = controller.signal.aborted ? 'timeout' : 'failed';
          row.errorCode = controller.signal.aborted ? 'timeout' : 'request-failed';
        } finally {
          active = false;
          clearTimeout(timer);
          controller.abort();
          this.pending = undefined;
        }
        row.end = { ...this.ports.context() };
        if (row.start.route !== row.end.route || row.start.path !== row.end.path) { row.status = 'invalid'; row.errorCode = 'route-changed'; }
        this.rows = [...this.rows.slice(-(C6_LIMIT - 1)), row];
        this.ports.changed();
      }
    } finally { this.busy = false; this.ports.changed(); }
  }
}
