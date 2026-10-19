import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Opt-in development scenario replay',
  summary: 'In the development server, add ?scenario=<catalog-id> to replay shared scenario client-v0 frames and arrangement folders through the real SDK and live data source without connecting to a gateway. The selected world stays in navigation URLs and replay folder transports survive hot reloads. Without the query startup stays live, and replay code and worlds are excluded from production builds.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
