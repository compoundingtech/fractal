import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Close tooltips with the pointer that opened them',
  summary: 'A tooltip closes when the pointer leaves its control unless that control has keyboard focus, and stays closed for controls covered by a dialog.',
  kind: 'fix',
} satisfies ChangelogEntrySource
