import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Type directly in agent terminals',
  summary: 'Click a terminal and type: keys, input methods and pastes go straight to the shell, and output appears as it happens. A chip shows whether you are typing, reconnecting or read-only. If the raw connection fails, authorized typing falls back to the gateway. Read-only devices can still watch. Returning from a hidden tab requires a fresh click or focus before typing resumes.',
  kind: 'feature',
} satisfies ChangelogEntrySource
