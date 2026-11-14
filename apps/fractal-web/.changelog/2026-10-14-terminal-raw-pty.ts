import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Type directly in agent terminals',
  summary: 'Click a raw PTY terminal to type directly into the shell with keys, input methods and pastes while output appears as it happens. A chip shows whether you are typing, reconnecting or read-only; authorized typing falls back to the gateway if the raw connection fails, while read-only sessions stay read-only and can still watch. Returning from a hidden tab requires a fresh click or focus before typing resumes.',
  kind: 'feature',
} satisfies ChangelogEntrySource
