import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Harness events show as a neutral notice card',
  summary: 'Harness events in a conversation, such as background jobs, compaction and IRC messages, now appear as a quiet notice card in the transcript. The original event details stay available through an explicit disclosure.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
