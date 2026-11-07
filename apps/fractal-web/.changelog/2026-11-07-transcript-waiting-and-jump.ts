import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Clearer waiting and a way back to a running reply',
  summary: 'A conversation waiting for your answer now shows an amber dot instead of a stopped spinner. If you scroll up while an agent is working, a Working, Jump to latest button takes you back to the end.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
