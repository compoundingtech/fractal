import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'The sidebar holds everything app-wide',
  summary: 'The connection and Usage now sit at the bottom of the sidebar, "Layout not syncing" sits under the space switcher, and a refused answer shows as a note at the bottom left instead of a band in a pane. Collapsing the sidebar leaves a narrow strip of icons with their counts, so the space, search, Settings and a lost connection stay one click away. When the connection is gone, the pane you are in says so too, and on a phone the button that opens the sidebar shows a dot.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
