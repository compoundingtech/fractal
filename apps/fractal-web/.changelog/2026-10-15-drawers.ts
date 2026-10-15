import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'The thread fills narrow windows',
  summary: 'On windows narrower than 900 px the thread header keeps the agent name, its status and one Agent details button. That button opens a sheet from the right with the agent’s actions above Changes or the open tool, instead of squeezing the thread beside an inline panel. Escape closes the sheet.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
