import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep question navigation clear',
  summary: 'Pending questions remain named groups without adding repeated navigation landmarks. Long question tabs show an ellipsis instead of cutting words, while screen readers retain the complete question. Open terminal is the only action while web answers are unavailable, and local selections are not sent.',
  kind: 'fix',
} satisfies ChangelogEntrySource
