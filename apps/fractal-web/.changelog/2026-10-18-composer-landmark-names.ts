import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Identify message composers by conversation',
  summary: 'Message composer landmarks include their conversation label so screen-reader users can distinguish composers in split panes. Message fields and send controls keep their existing names.',
  kind: 'fix',
} satisfies ChangelogEntrySource
