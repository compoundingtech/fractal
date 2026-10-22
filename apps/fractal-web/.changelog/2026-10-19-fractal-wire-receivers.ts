import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keeps quota readings compatible with Fractal wire names',
  summary: 'The monitor accepts both existing and Fractal response version names, keeping quota readings available across the rename.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
