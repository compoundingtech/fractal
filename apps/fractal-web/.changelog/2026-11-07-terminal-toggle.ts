import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Swap between the conversation and the terminal',
  summary: 'The header control now switches the main pane between an agent\'s conversation and its terminal instead of opening a drawer. The terminal stays connected while hidden, so switching back shows it at once, and a reload restores the view.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
