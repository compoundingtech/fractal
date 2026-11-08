// The simplified conversation, as fractal-conversation-ui draws it (Density::Simple): a tool call is
// one line, and a run of calls in a row is one line saying how many and how they went until it
// is opened. Everything else reads as in full. The phone shows it by default; the choice is the
// device's own.
import type { ConversationEntry } from './conversationView.ts';

type Tool = Extract<ConversationEntry['body'], { kind: 'tool' }>;
export type SimpleRow =
  | { kind: 'entry'; entry: ConversationEntry }
  | { kind: 'call'; entry: ConversationEntry; tool: Tool }
  | { kind: 'bundle'; id: string; calls: Array<{ entry: ConversationEntry; tool: Tool }>; ok: number; failed: number; running: number; last: string; open: boolean };

/** The id that opens a run of calls starting at `first`, as the Fractal terminal UI's `bundle_id`. */
export const bundleId = (first: string) => `bundle:${first}`;

/** The tool call an entry folds into a run as, or undefined when it reads as in full. */
const foldedCall = (entry: ConversationEntry): Tool | undefined => {
  const body = entry.body;
  if (body.kind !== 'tool' || body.title.startsWith('assistant error') || body.title.startsWith('compaction') || body.title.startsWith('write ')) return undefined;
  return body;
};

/** Entries, oldest first, as simplified rows; `open` holds the ids of opened runs. */
export function simplify(entries: readonly ConversationEntry[], open: ReadonlySet<string>): SimpleRow[] {
  const rows: SimpleRow[] = [];
  let calls: Array<{ entry: ConversationEntry; tool: Tool }> = [];
  const closeRun = () => {
    const first = calls[0];
    const last = calls.at(-1);
    if (first === undefined || last === undefined) return;
    if (calls.length === 1) {
      rows.push({ kind: 'call', ...first });
    } else {
      const run = calls;
      const id = bundleId(first.entry.id);
      const count = (state: Tool['state']) => run.filter(call => call.tool.state === state).length;
      rows.push({ kind: 'bundle', id, calls: run, ok: count('ok'), failed: count('failed'), running: count('running'), last: last.tool.title, open: open.has(id) });
      if (open.has(id)) for (const call of run) rows.push({ kind: 'call', ...call });
    }
    calls = [];
  };
  for (const entry of entries) {
    const tool = foldedCall(entry);
    if (tool !== undefined) {
      calls.push({ entry, tool });
      continue;
    }
    closeRun();
    rows.push({ kind: 'entry', entry });
  }
  closeRun();
  return rows;
}
