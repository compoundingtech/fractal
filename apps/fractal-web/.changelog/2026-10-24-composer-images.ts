import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Send images from the composer',
  summary: 'Attach or paste up to four PNG, JPEG, GIF or WebP images, each at most 10 MiB. They upload when you send; rejected files explain how to recover, and staged images stay removable if upload permission changes.',
  kind: 'feature',
} satisfies ChangelogEntrySource
