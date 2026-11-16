import type { QualityStorage } from './quality';

export const DRAFTS_KEY = 'st3.drafts.local.v1';
const MAX_BYTES = 2 * 1024 * 1024;
/** Draft content has its own local key; it never enters a quality summary. */
export class LocalDrafts {
  private issue: string | null = null;
  private listeners = new Set<() => void>();
  snapshot = (): string | null => this.issue;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private report(issue: string | null) { if (this.issue === issue) return; this.issue = issue; for (const listener of this.listeners) listener(); }
  private writing: Promise<void> | null = null;
  private pendingOwner = '';
  private pendingDrafts: ReadonlyMap<string, string> | null = null;
  private clearing = false;
  constructor(private readonly storage: QualityStorage) {}
  async restore(owner: string, into: Map<string, string>, current: () => boolean = () => true): Promise<number> {
    let raw: string | null;
    try { raw = await this.storage.getItem(DRAFTS_KEY); } catch { return 0; }
    if (!current() || !raw || raw.length > MAX_BYTES) return 0;
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!parsed || typeof parsed !== 'object' || !('owner' in parsed) || parsed.owner !== owner || !('entries' in parsed) || !Array.isArray(parsed.entries)) return 0;
      let restored = 0;
      for (const entry of parsed.entries) {
        if (!Array.isArray(entry) || entry.length !== 2 || typeof entry[0] !== 'string' || typeof entry[1] !== 'string' || !entry[1] || into.has(entry[0])) continue;
        into.set(entry[0], entry[1]); restored++;
      }
      return restored;
    } catch { return 0; }
  }
  /** Called after a composer commit or on background, never in its key handler. */
  save(owner: string, drafts: ReadonlyMap<string, string>): Promise<void> {
    if (!owner) return this.writing ?? Promise.resolve();
    this.pendingOwner = owner; this.pendingDrafts = drafts; this.clearing = false;
    return this.drain();
  }
  clear(): Promise<void> { this.pendingDrafts = null; this.clearing = true; return this.drain(); }
  private drain(): Promise<void> {
    if (!this.writing) this.writing = Promise.resolve().then(() => this.writePending());
    return this.writing;
  }
  private async writePending(): Promise<void> {
    // One write in flight plus the current Map reference. Edits while the disk
    // writes replace the pending view; no queue of serialized draft snapshots.
    while (this.pendingDrafts || this.clearing) {
      const drafts = this.pendingDrafts, owner = this.pendingOwner;
      this.pendingDrafts = null; this.clearing = false;
      const raw = drafts ? JSON.stringify({ owner, entries: [...drafts] }) : '';
      if (new TextEncoder().encode(raw).length > MAX_BYTES) {
        this.report('Draft storage is full. Your current drafts remain in this app session, but recent changes are not saved for reopening. Send or clear drafts to make room.');
      } else {
        try { await this.storage.setItem(DRAFTS_KEY, raw); this.report(null); }
        catch { this.report('Drafts could not be saved on this device. Keep the app open to retain your current notes.'); }
      }
    }
    this.writing = null;
  }
}
