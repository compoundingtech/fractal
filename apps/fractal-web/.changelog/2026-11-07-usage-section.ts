import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See account quota at the bottom of the sidebar',
  summary: 'A collapsible Usage section shows the worst account in one line and, when open, every account’s window, meter, percentage and reset, worst first; a reading at its limit is marked with “!” as well as colour. The section remembers whether it is open and how tall it is through the shared preferences document, while embeds and stories keep isolated choices. It says so plainly when no quota source is configured.',
  kind: 'feature',
} satisfies ChangelogEntrySource
