import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Identify agents in browser tabs',
  summary: 'Browser tabs show the selected agent, offline state and the number of agents needing attention. The favicon highlights working, attention and error states. Leaving the workspace restores the original tab title and favicon.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
