import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Answer a prompt only beside the call it asks about',
  summary: 'An alert for a terminal prompt offers allow, approve or answer only when Fractal names the call the prompt asks about. When it does not, the alert says so and offers only deny, or answering in the terminal, instead of guessing which call is meant.',
  kind: 'fix',
} satisfies ChangelogEntrySource
