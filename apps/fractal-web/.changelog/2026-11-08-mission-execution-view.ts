import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'A mission’s tab shows how its run is going',
  summary: 'A mission tab now opens on an overview of progress, time, spend, what runs now and what needs you, then the plan as a dependency graph over a wall-clock trace. Each trace row says who ran the step and what it cost, every span shows its duration, and the playhead on the time axis replays the run. Hovering a step anywhere shows it in the inspector, which lives in the tab’s side panel next to Plan and Agents. A Definition tab shows the mission’s KDL, with a place for its TypeScript source.',
  kind: 'feature',
} satisfies ChangelogEntrySource
