import type { Snapshot, ThreadSummary } from "../../clients/typescript/fractal-client";

export function threadRefusalMessage(reason: string): string {
  return `You no longer have access to this thread.${reason ? ` ${reason}${/[.!?]$/.test(reason) ? "" : "."}` : ""}`;
}

export type ThreadEdit = { kind: "rename"; title: string } | { kind: "close" | "reopen" };
export type ThreadEditResult =
  | { how: "sent"; snapshot?: Snapshot }
  | { how: "failed"; error: string; action?: ThreadEdit["kind"] }
  | { how: "unconfirmed"; retry: () => Promise<ThreadEditResult> };
type Summary = Pick<ThreadSummary, "id" | "title" | "state">;
type Patch = Partial<Pick<Summary, "title" | "state">>;
type Presentation = { patch: Patch; phase: "pending" | "sent"; snapshot?: Snapshot; flash?: string };

/** One connection's local edits; authoritative summaries and request fences stay untouched. */
export class ThreadEdits {
  private readonly entries = new Map<string, Presentation>();
  constructor(
    private readonly changed: () => void,
    private readonly errorText: (error: unknown) => string,
  ) {}

  flash(id: string): string | undefined {
    return this.entries.get(id)?.flash;
  }

  project<T extends Summary>(thread: T): T {
    const entry = this.entries.get(thread.id);
    if (!entry || (entry.patch.title === undefined && entry.patch.state === undefined)) return thread;
    return { ...thread, ...entry.patch };
  }

  /** A same-host snapshot at/after the receipt also covers edits whose intermediate value was skipped. */
  observe(thread: Summary, snapshot?: Snapshot): void {
    const entry = this.entries.get(thread.id);
    if (!entry || entry.phase !== "sent" || (entry.patch.title === undefined && entry.patch.state === undefined))
      return;
    if (entry.snapshot) {
      if (!snapshot || snapshot.host_id !== entry.snapshot.host_id || snapshot.store_index < entry.snapshot.store_index)
        return;
      entry.patch = {};
      if (!entry.flash) this.entries.delete(thread.id);
      this.changed();
      return;
    }
    const titleMatches = entry.patch.title !== undefined && thread.title === entry.patch.title;
    const stateMatches = entry.patch.state !== undefined && thread.state === entry.patch.state;
    if (!titleMatches && !stateMatches) return;
    if (titleMatches) delete entry.patch.title;
    if (stateMatches) delete entry.patch.state;
    if (!entry.flash && entry.patch.title === undefined && entry.patch.state === undefined)
      this.entries.delete(thread.id);
    this.changed();
  }

  async run(thread: Summary, edit: ThreadEdit, send: () => Promise<ThreadEditResult>): Promise<ThreadEditResult> {
    if (this.entries.get(thread.id)?.phase === "pending") {
      return { how: "failed", action: edit.kind, error: "Resolve the earlier change before starting another." };
    }
    const title = this.project(thread).title;
    const priorPatch = { ...this.entries.get(thread.id)?.patch };
    const priorSnapshot = this.entries.get(thread.id)?.snapshot;
    const patch: Patch = {
      ...priorPatch,
      ...(edit.kind === "rename" ? { title: edit.title } : { state: edit.kind === "close" ? "closed" : "open" }),
    };
    const entry: Presentation = { patch, phase: "pending" };
    this.entries.set(thread.id, entry);
    this.changed();
    const finish = async (operation: () => Promise<ThreadEditResult>): Promise<ThreadEditResult> => {
      let result: ThreadEditResult;
      try {
        result = await operation();
      } catch (error) {
        result = { how: "failed", error: this.errorText(error) };
      }
      if (result.how === "unconfirmed") {
        const retry = result.retry;
        return { how: "unconfirmed", retry: () => finish(retry) };
      }
      if (result.how === "failed") {
        if (priorPatch.title !== undefined || priorPatch.state !== undefined)
          this.entries.set(thread.id, { patch: priorPatch, snapshot: priorSnapshot, phase: "sent" });
        else this.entries.delete(thread.id);
        result = { ...result, action: edit.kind };
      } else {
        entry.phase = "sent";
        entry.snapshot = result.snapshot;
        entry.flash =
          edit.kind === "rename"
            ? `Renamed to "${edit.title}"`
            : edit.kind === "close"
              ? `Closed "${title}" · Reopen thread to undo.`
              : `Reopened "${title}"`;
        setTimeout(() => {
          if (this.entries.get(thread.id) !== entry) return;
          entry.flash = undefined;
          if (entry.patch.title === undefined && entry.patch.state === undefined) this.entries.delete(thread.id);
          this.changed();
        }, 4000);
      }
      this.changed();
      return result;
    };
    return finish(send);
  }
}
