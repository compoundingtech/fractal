import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Clearer diff colors',
  summary: 'Light-mode diffs use darker blue addition markers, red removal markers, and amber syntax literals so changed lines remain readable against their background washes. Other code previews retain their existing syntax colors.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
