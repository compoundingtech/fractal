import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Task results and harness notices show as compact cards',
  summary: 'Task results, IRC messages, background-job completions and context compaction from OMP agents now appear as compact neutral cards instead of raw envelope text. Open a card to read the full payload; ordinary messages are left as written.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
