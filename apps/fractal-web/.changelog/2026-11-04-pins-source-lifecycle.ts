import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Scope Pins follows to their data source',
  summary: 'Pins reads use the selected source transport. Switching sources, retrying or leaving the workspace cancels initializing reads and closes their subscriptions. This prepares the sidebar binding without enabling new controls.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
