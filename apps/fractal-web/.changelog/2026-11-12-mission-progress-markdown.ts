import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Mission progress reads as formatted text',
  summary: 'A mission’s latest progress shows its Markdown formatting in the Missions pane, and one-line labels such as sidebar rows, mission titles and waiting questions show plain words without Markdown markers.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
