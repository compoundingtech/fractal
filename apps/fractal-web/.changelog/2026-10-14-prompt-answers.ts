import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Answer an agent’s prompts from its conversation',
  summary: 'When an agent waits on a permission, a question, a plan or a setup prompt, its conversation shows what it asks and lets you answer here. Allow and approve appear only beside the call they would run, shown in full. If st cannot deliver an answer, the alert says to answer in the agent’s terminal.',
  kind: 'feature',
} satisfies ChangelogEntrySource
