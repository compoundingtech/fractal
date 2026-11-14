import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep response usage evidence faithful',
  summary: 'Response details retain observed model and provider identity, timing, reasoning, cache usage and provider quota. Usage summaries distinguish unknown response prices from reported zero cost, including responses without a token total.',
  kind: 'fix',
} satisfies ChangelogEntrySource
