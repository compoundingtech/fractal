import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Needs me filter name matches its label',
  summary: 'The sidebar Needs me filter is now announced with the same text it shows, including the count, so screen readers and voice control can find it by what is on screen.',
  kind: 'fix',
} satisfies ChangelogEntrySource
