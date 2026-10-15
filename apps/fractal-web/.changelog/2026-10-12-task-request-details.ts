import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Task requests read like instructions',
  summary: 'Delegated tasks show their names, agent types, and formatted instructions instead of quoted data. Their launch status is labeled as a snapshot, and the inline result preview is readable too.',
  kind: 'fix',
} satisfies ChangelogEntrySource
