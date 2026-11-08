import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show a re-delivered prompt once',
  summary: 'When an agent is given the same message again, the transcript shows that prompt once and says how many times the agent received it. Only a delivery link from Fractal merges prompts, never matching text.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
