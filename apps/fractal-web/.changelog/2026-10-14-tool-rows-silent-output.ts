import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Silent tool rows stay quiet',
  summary: 'Tool rows no longer add a No output note when a call returned nothing. Empty inputs and image-only results stay quiet, including when the conversation supplies a detail renderer. Rows open only for readable output, non-empty recorded raw input or actual host-provided detail; input-only calls show that input without an empty output block.',
  kind: 'fix',
} satisfies ChangelogEntrySource
