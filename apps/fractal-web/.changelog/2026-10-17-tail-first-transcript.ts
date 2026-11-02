import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Long conversations open at the latest messages',
  summary: 'Short conversations remain fully shown, while long conversations open at their latest turns and reveal older turns without losing your reading position, with persistent Show earlier messages, Show all messages, and browser Find scope controls. Show all messages, Ctrl/Cmd-F, and links progressively prepare older history in small batches; browser Find covers only shown messages until preparation finishes, and replaced or removed links cancel their old jump. History preparation and prompt measurements pause in inactive panes and resume when you return while incoming updates are retained; timestamps reuse their formatter and unchanged Markdown avoids reparsing.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
