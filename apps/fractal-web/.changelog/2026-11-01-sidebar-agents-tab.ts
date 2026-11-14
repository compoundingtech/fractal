import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read agent status in the sidebar',
  summary: 'The sidebar now labels its Agents view with an attention marker before the tab name, equal-width tabs, and full touch targets. Agent rows use shared activity glyphs and show a diamond when they need you or a question mark for a harness question. Press G then A to open Agents, or the question-mark key to see navigation shortcuts even when the sidebar is collapsed.',
  kind: 'feature',
} satisfies ChangelogEntrySource
