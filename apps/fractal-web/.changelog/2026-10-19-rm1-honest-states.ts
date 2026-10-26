import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Honest stale and unavailable states',
  summary: 'Disconnected rosters and conversations keep their last observed content without implying that work is still live, and show observation ages when the source provides them. Failed roster and conversation reads offer Retry, while loading, empty and no-match views use the redesign milestone presentation.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
