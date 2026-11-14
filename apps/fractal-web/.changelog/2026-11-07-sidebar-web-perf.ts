import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Bound inactive Sidebar per-input projection work',
  summary: 'Indexes complete membership models once, reuses projections across unrelated snapshot writes, and measures cached input planning plus pending updates against the v1 component baseline. The current Sidebar is unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
