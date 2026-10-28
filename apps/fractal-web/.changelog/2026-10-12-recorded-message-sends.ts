import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Recorded agent messages keep their meaning',
  summary: 'Recorded conversation-send commands show the destination and formatted message instead of appearing as ordinary commands. Quoted mentions of those commands stay ordinary tool work.',
  kind: 'fix',
} satisfies ChangelogEntrySource
