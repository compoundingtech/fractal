import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read older messages without losing your place',
  summary: 'Long conversations render a small window of messages. Scroll to the beginning to load older history; your reading position stays put, and a failed page can be retried without losing the messages already loaded.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
