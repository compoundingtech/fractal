import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Prepare ordered Sidebar move planning',
  summary: 'Adds inactive membership move planning with exact canonical ordering keys, minimal local rekeys, cycle and no-op refusals, and bounded edit payloads. The current Sidebar is unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
