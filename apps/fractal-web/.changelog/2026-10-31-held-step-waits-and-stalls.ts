import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See what a held step waits on, or that it stalled',
  summary: "An agent's status now leads with what its held step waits on, such as a pull request or a time, or says the step stalled: no progress and no declared wait.",
  kind: 'feature',
} satisfies ChangelogEntrySource
