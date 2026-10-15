import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Tool details keep recorded evidence exact',
  summary: 'Raw tool input and output show exactly what was recorded, and JSON is only reindented when nothing would change. Wait details name the sender who replied and list only completed jobs as arrivals, while other jobs stay labelled Still running. A result with an unfamiliar view stays joined to its call, and going Back to another conversation closes the previous tool inspector.',
  kind: 'fix',
} satisfies ChangelogEntrySource
