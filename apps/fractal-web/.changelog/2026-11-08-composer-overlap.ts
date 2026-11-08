import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'The conversation runs under the composer',
  summary: 'The empty band between the conversation and the message box is gone: the conversation scrolls under the composer behind a short fade, and at the end its last line sits fully above it. "Jump to latest" now shows whenever you have scrolled up, and adds how many new messages arrived.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
