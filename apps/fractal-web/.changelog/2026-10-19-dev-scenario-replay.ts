import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Opt-in development scenario replay',
  summary: 'In the development server, add ?scenario=<catalog-id> to replay shared scenario client-v0 frames through the real SDK and live data source without connecting to a gateway. Without the query, startup stays live. Replay code and worlds are excluded from production builds.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
