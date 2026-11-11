import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Build details and changes in one place',
  summary: 'The developer bar shows the build revision and age as one entry. Opening it shows a one-line build summary above recent changes, with full build details one click away. Its dot marks unseen changes and clears once the newest change is on screen.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
