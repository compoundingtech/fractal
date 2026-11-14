import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: "Recover conversation and terminal chunk failures honestly",
  summary: "Conversation and terminal views retry once in the same document, then offer Reload and retry after a second rejection without automatically discarding drafts or selection.",
  kind: 'improvement',
} satisfies ChangelogEntrySource
