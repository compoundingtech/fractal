import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Finished work reads as one line',
  summary: 'A finished turn shows one collapsed line such as "Worked 6s · 5 tools". Its rows open only when you click them, and a failed tool row stays in view. An open row no longer repeats its title, and Cmd or Ctrl+F in a focused terminal stays with the terminal.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
