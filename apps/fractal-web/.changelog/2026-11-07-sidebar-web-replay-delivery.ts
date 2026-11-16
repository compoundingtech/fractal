import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Preserve uncertain Sidebar membership writes',
  summary: 'Inactive journal-backed membership writes now retain uncertain delivery and completed folder progress when a replay is refused before receipt lookup. First-delivery refusals remain definitive, and the current Sidebar is unchanged.',
  kind: 'fix',
} satisfies ChangelogEntrySource
