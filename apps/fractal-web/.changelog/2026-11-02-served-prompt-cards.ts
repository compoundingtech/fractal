import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read question cards from served attention',
  summary: 'Prompt cards now show the calls and options Fractal serves with the open attention row instead of inferring them from conversation history. Missing, incomplete or unsupported prompt details explain that the answer belongs in the agent’s terminal.',
  kind: 'fix',
} satisfies ChangelogEntrySource
