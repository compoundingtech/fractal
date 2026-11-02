import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep question navigation clear',
  summary: 'Pending OMP questions remain named groups without adding repeated navigation landmarks. Long question tabs show an ellipsis instead of cutting words, while screen readers retain the complete question.',
  kind: 'fix',
} satisfies ChangelogEntrySource
