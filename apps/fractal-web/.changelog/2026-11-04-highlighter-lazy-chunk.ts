import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: "Keep code readable when highlighting cannot load",
  summary: "If syntax highlighting fails to load, code stays plain for the session without retry or reload controls. Conversation and terminal recovery is unchanged.",
  kind: 'fix',
} satisfies ChangelogEntrySource
