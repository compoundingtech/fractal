import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Long conversations open at the latest messages',
  summary: 'Long transcripts open at the latest turns and reveal older turns as you scroll up, keeping your reading position steady. A persistent history bar keeps Show earlier messages, Show all messages, and the browser Find search scope visible at the latest messages. Show all messages and Ctrl/Cmd-F progressively prepare the full conversation in small batches while preserving your reading position; browser Find searches only currently shown messages until preparation finishes, so search again afterward. Links to older messages progressively reveal their target, and removing or replacing a link cancels the old jump. Full-history preparation pauses in inactive retained panes while incoming messages and streaming updates are retained; returning resumes preparation without moving your reading position. Prompt measurements run in small batches and pause for inactive retained panes, then refresh when you return. Timestamps reuse their formatter, and unchanged Markdown avoids parsing again when surrounding data updates.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
