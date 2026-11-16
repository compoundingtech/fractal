import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Review the real workspace in every availability state',
  summary: 'The component reference now shows the production agent workspace and conversation pane while loading, empty, populated, after a failed tool, offline, unavailable and during a streaming repair session. Each view runs the same modules the app ships, including the offline connection banner, over fictional fixture data, and never connects to a gateway.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
