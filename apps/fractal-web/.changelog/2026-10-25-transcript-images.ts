import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'View images in conversations',
  summary: 'Choose Show image on a sent or received attachment to read it with your viewer permissions, then open the thumbnail in the image viewer. Failed reads can be retried; images never load automatically. File sizes appear when the message carries them.',
  kind: 'feature',
} satisfies ChangelogEntrySource
