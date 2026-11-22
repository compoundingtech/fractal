import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Act on a stuck mission',
  summary: 'The actions Fractal offers on a mission now work from the web: retry a failed step, start or restart its agent, or cancel the run. Cancelling and restarting ask first and say what will happen. Each shows that it is on its way, then that it was sent or why it was not.',
  kind: 'feature',
} satisfies ChangelogEntrySource
