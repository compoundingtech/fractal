import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Tool rows keep the agent’s intent',
  summary: 'When the harness records why a tool is being used, that intent is the row title. The action name and arguments remain secondary, and calls without a recorded intent fall back to the same readable action vocabulary.',
  kind: 'fix',
} satisfies ChangelogEntrySource
