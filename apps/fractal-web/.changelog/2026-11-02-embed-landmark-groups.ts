import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep repeated conversation controls accessible',
  summary: 'Work logs and message composers are named groups within their workspace rather than repeated page landmarks. Pages showing multiple product embeds retain their labels and form behavior without duplicate landmarks.',
  kind: 'fix',
} satisfies ChangelogEntrySource
