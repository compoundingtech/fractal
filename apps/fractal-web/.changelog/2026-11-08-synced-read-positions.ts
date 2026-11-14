import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Unread activity tracks conversation prose',
  summary: 'Agent projections now expose the latest conversation prose timestamp, so unread indicators can distinguish new words from tool activity. Synced read positions are shared by person across supporting devices; the web sync wiring is delivered separately.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
