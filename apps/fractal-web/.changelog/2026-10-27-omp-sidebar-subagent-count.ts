import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See OMP subagent counts in the live sidebar',
  summary: 'OMP agents now show a compact subagent count chip in the live sidebar, with a tooltip explaining missing reports. Stale observations retain their last-verified count and children. Harnesses that do not report subagents show no chip or invented zero count.',
  kind: 'feature',
} satisfies ChangelogEntrySource
