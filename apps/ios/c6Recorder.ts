// C6 is opt-in, device-local, and never part of observation reporting.
export type C6Context = { route: 'tailscale' | 'lan' | 'https' | 'fabric' | 'none'; path: 'direct' | 'relay' | 'unknown' };
export type C6Snapshot = { identity: object | null; context: C6Context };
type C6ObservedContext = { route: C6Context['route']; observedPath: C6Context['path'] };
export type C6Row = {
  runId: string; blockId: string; index: number; operation: 'status' | 'send-echo';
  version: string; start: C6ObservedContext; end: C6ObservedContext; pathScope: 'module-global' | 'unknown'; pathFlag: 'changed' | 'unknown' | null; requestId: string | null; messageId: string | null;
  ackMs: number | null; echoMs: number | null; status: 'ok' | 'failed' | 'timeout' | 'invalid' | 'cancelled';
  errorCode: 'request-failed' | 'timeout' | 'route-changed' | 'cancelled' | null;
  errorName: string | null;
};
export type C6Send = { signal: AbortSignal; request: (id: string) => void; ack: () => void; cleanup: (close: () => void) => void };
export const C6_LIMIT = 200;
export const C6_TIMEOUT_MS = 15_000;
export const nearestRank = (values: readonly number[], percentile: number): number | null => {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.min(sorted.length - 1, Math.ceil(percentile * sorted.length) - 1))];
};
export class C6Recorder<TSample extends C6Snapshot = C6Snapshot> {
  rows: readonly C6Row[] = [];
  busy = false;
  private cancelled = false;
  private controller?: AbortController;
  private pending?: { nonce: string; observed: (id: string) => void };
  constructor(private ports: { now: () => number; id: () => string; snapshot: () => TSample; readFabricPath: () => Promise<C6Context['path']>; version: string; changed: () => void; timeoutMs?: number }) {}
  clear(): void { if (!this.busy) { this.rows = []; this.ports.changed(); } }
  cancel(): void { if (this.busy) { this.cancelled = true; this.controller?.abort(); } }
  observe(id: string, text: string, own: boolean): void {
    const pending = this.pending;
    if (own && pending && text.includes(`[C6:${pending.nonce}]`)) pending.observed(id);
  }
  export(): string { return JSON.stringify({ schema: 'ios-c6.v2', rows: this.rows }, null, 2); }
  async run(operation: C6Row['operation'], execute: (nonce: string, hooks: C6Send, sample: TSample) => Promise<void>): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.cancelled = false;
    this.ports.changed();
    const runId = this.ports.id(), blockId = this.ports.id();
    try {
      for (let index = 1; index <= 10; index++) {
        // Capture transport and its label together; execute receives this exact snapshot.
        const sample = { ...this.ports.snapshot() }, start: C6ObservedContext = { route: sample.context.route, observedPath: sample.context.path };
        if (start.route === 'fabric' && !this.cancelled) start.observedPath = await this.ports.readFabricPath();
        const startRead = this.ports.snapshot();
        const began = this.ports.now();
        const row: C6Row = { runId, blockId, index, operation, version: this.ports.version, start, end: start, pathScope: start.route === 'fabric' ? 'module-global' : 'unknown', pathFlag: null, requestId: null, messageId: null, ackMs: null, echoMs: null, status: 'ok', errorCode: null, errorName: null };
        if (this.cancelled) {
          row.status = 'cancelled'; row.errorCode = 'cancelled';
        } else {
          const controller = new AbortController(), nonce = this.ports.id();
          this.controller = controller;
          let timer: ReturnType<typeof setTimeout> | undefined;
          let active = true;
          let cleanup: (() => void) | undefined;
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
                  cleanup: close => { cleanup = close; },
                }, sample);
                if (operation === 'send-echo') await echo;
              })(),
              new Promise<never>((_, reject) => {
                controller.signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true });
                if (controller.signal.aborted) reject(new Error('aborted'));
                timer = setTimeout(() => controller.abort(), this.ports.timeoutMs ?? C6_TIMEOUT_MS);
              }),
            ]);
          } catch (error) {
            row.errorName = error instanceof Error ? error.name : null;
            row.status = this.cancelled ? 'cancelled' : controller.signal.aborted ? 'timeout' : 'failed';
            row.errorCode = this.cancelled ? 'cancelled' : controller.signal.aborted ? 'timeout' : 'request-failed';
          } finally {
            active = false;
            clearTimeout(timer);
            controller.abort();
            cleanup?.();
            this.controller = undefined;
            this.pending = undefined;
          }
          const end = this.ports.snapshot();
          row.end = { route: end.context.route, observedPath: end.context.path };
          if (row.end.route === 'fabric') row.end.observedPath = await this.ports.readFabricPath();
          const endRead = this.ports.snapshot();
          if (this.cancelled) { row.status = 'cancelled'; row.errorCode = 'cancelled'; } else {
            if (sample.identity !== startRead.identity || sample.identity !== end.identity || sample.identity !== endRead.identity || row.start.route !== startRead.context.route || row.start.route !== row.end.route || row.end.route !== endRead.context.route) {
              row.status = 'invalid'; row.errorCode = 'route-changed';
            }
            if (row.start.route === 'fabric' && (row.start.observedPath === 'unknown' || row.end.observedPath === 'unknown')) {
              row.pathFlag = 'unknown';
            } else if (row.start.route === 'fabric' && row.start.observedPath !== row.end.observedPath) {
              row.pathFlag = 'changed';
            }
          }
        }
        this.rows = [...this.rows.slice(-(C6_LIMIT - 1)), row];
        this.ports.changed();
      }
    } finally { this.busy = false; this.ports.changed(); }
  }
}
