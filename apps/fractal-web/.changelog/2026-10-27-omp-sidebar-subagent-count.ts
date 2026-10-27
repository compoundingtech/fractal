import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See OMP subagent counts in the live sidebar',
  summary: 'OMP agents now show a subagent count chip in the live sidebar. The chip shows an unknown state when the report is missing or stale. Other harnesses do not show a chip.',
  kind: 'feature',
} satisfies ChangelogEntrySource
