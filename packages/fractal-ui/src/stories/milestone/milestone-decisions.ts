/**
 * Redesign milestone 1 (RM1): the decisions this milestone composes.
 *
 * Every row is decided; none is re-asked. Question IDs are stable: a later revision may add IDs but never reuses or
 * renumbers one. `question` links a row to the decision request that settled it.
 */

export interface LockedDecision { readonly id: string; readonly summary: string; readonly question?: string }

export const lockedDecisions: readonly LockedDecision[] = [
  { id: 'SR-2', summary: 'Sidebar agent row: 48px two-line row. Line 1: status, title, last turn. Line 2: status word (when not working), host, spend, duration.' },
  { id: 'WF-1', summary: 'Work log: routine tool calls fold into one "Worked for" row; edits and failures stay visible.' },
  { id: 'RC-1', summary: 'Resource card: 40px flat wash card that opens the resource in a split. Composed here as the subagent result card in the parent answer.' },
  { id: 'RC-3', summary: 'File chips inside prose only (baseline-aligned, hover preview).' },
  { id: 'SG-1', summary: 'Status glyph: 8px disc in a fixed 16px column; no extra leading pip or dot.' },
  { id: 'RM1-Q1', summary: 'Rows stay two lines at every sidebar width (no one-line fold).' },
  { id: 'RM1-Q2', summary: 'Roster loading shows row-shaped placeholders at the real 48px row pitch.' },
  { id: 'RM1-Q3', summary: 'A roster failure appears inside the sidebar, where the list would be.' },
  { id: 'RM1-Q4', summary: 'With no agent open, the workspace shows one line plus keyboard hints.' },
  { id: 'RM1-Q5', question: 'Q135', summary: 'Mod+K searches agents first, then commands, in one list.' },
  { id: 'RM1-Q6', question: 'Q137', summary: 'SG-1 shape cues (G3): states other than working and idle add a shape (needs you ?, stale clock, ended square), so states read without colour.' },
  { id: 'RM1-Q7', question: 'Q136', summary: 'Inbox, Missions and Resources sit in one compact row under the agent list.' },
  { id: 'Composer', summary: 'Not part of this milestone. The composer is the current one, visually unchanged, until its scope is revisited.' },
]
