import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'A mission’s tab shows how its run is going',
  summary: 'A mission tab now opens on an overview of progress, time, spend, what runs now and what needs you, then the plan as a dependency graph over a wall-clock trace whose rows say who ran each step and what it cost, with every span’s duration and a playhead on the time axis that replays the run. Hovering a step anywhere shows it in the inspector, which lives in the tab’s side panel next to Plan and Agents. A Definition tab shows the mission’s KDL, with a place for its TypeScript source.',
  kind: 'feature',
} satisfies ChangelogEntrySource
