import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'New windows keep your open tabs',
  summary: 'Opening a new web window no longer clears the tabs that another window has open. A new window saves its tabs only after you change them.',
  kind: 'fix',
} satisfies ChangelogEntrySource
