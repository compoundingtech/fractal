import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read agent status in the sidebar',
  summary: 'The sidebar now labels its Agents view with an attention marker before the tab name. Agent rows use shared activity glyphs and show a diamond when they need you or a question mark for a harness question. Press G then A to open Agents, or ? to see navigation shortcuts.',
  kind: 'feature',
} satisfies ChangelogEntrySource
