import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Native transcripts hide internal records and show turn usage',
  summary: 'Conversations from native agents no longer show internal or harness-hidden payloads as messages or tool results, and records the app does not support appear as a marker with a redacted preview. When a turn reports its model, token usage and cost, they show once at the end of that turn.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
