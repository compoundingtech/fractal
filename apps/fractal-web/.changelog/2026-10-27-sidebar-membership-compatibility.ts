import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep Sidebar folders available with ordered membership',
  summary: 'Sidebar folders accept the upgraded arrangements capability while retaining existing layout support. Ordered-membership layouts are excluded from legacy Sidebar selection so they cannot receive incompatible drag actions.',
  kind: 'fix',
} satisfies ChangelogEntrySource
