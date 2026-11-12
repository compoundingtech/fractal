import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Changes panel uses the shared diff view',
  summary: 'The Changes panel shows each file with the same diff view as the transcript; unified or split layout, wrapping and hiding whitespace-only changes work as before.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
