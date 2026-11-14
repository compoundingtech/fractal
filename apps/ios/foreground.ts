// Background suspends gateway loops; active resumes them. Transient inactive states (a Share sheet,
// Control Center or an interruption) preserve the last state rather than tearing down live sockets.
export class ForegroundGate {
  private isActive: boolean;
  private waiters: Array<() => void> = [];
  private listeners = new Set<(active: boolean) => void>();

  constructor(initialState: string | null | undefined) { this.isActive = initialState === 'active'; }

  get active(): boolean { return this.isActive; }

  update(state: string | null | undefined): void {
    if (state !== 'active' && state !== 'background') return;
    const active = state === 'active';
    if (active === this.isActive) return;
    this.isActive = active;
    for (const listener of this.listeners) listener(active);
    if (active) { const waiters = this.waiters; this.waiters = []; for (const resume of waiters) resume(); }
  }

  untilActive(): Promise<void> {
    return this.isActive ? Promise.resolve() : new Promise(resolve => { this.waiters.push(resolve); });
  }

  subscribe(listener: (active: boolean) => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }
}
