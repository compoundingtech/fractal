import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'The thread fills narrow windows',
  summary: 'On windows narrower than 900 px the thread fills the screen: agents open as a sheet from the left, and one Agent details button in the header opens the agent’s actions above Changes or the open tool as a sheet from the right. Choosing an agent, Escape or a click outside closes either sheet. Wider windows are unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
