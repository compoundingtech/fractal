import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Agent rows show their mission and step',
  summary: 'Agent rows now show a compact mission pill beside the current step, with step counts when available. Questions waiting for you appear beside the mission, and queued missions open in a popover. Agents without a mission keep a simple status and host line.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
