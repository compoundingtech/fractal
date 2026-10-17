/**
 * Redesign milestone 1 (RM1): locked decisions and open questions.
 *
 * Locked rows are already-approved picks this milestone composes; they are not re-asked.
 * Question IDs are stable: a later revision may add IDs but never reuses or renumbers one.
 * Every option is a story arg, so each answer has its own deep link.
 */

export interface LockedDecision { readonly id: string; readonly summary: string }
export interface OpenQuestion<K extends string = string> {
  readonly id: string
  readonly arg: K
  readonly question: string
  /** The first option is the milestone default; the others are labelled alternatives. */
  readonly options: readonly { readonly value: string; readonly label: string }[]
}

export const lockedDecisions: readonly LockedDecision[] = [
  { id: 'SR-2', summary: 'Sidebar agent row: 48px two-line row (title + status/time; host, spend, duration on line 2).' },
  { id: 'WF-1', summary: 'Work log: routine tool calls fold into one "Worked for" row; edits and failures stay visible.' },
  { id: 'RC-1', summary: 'Resource card: 40px flat wash card that opens the resource in a split.' },
  { id: 'RC-3', summary: 'File chips inside prose only (baseline-aligned, hover preview).' },
  { id: 'SG-1', summary: 'Status glyph: 8px coloured disc in a fixed 16px column; no extra leading pip or dot.' },
  { id: 'Composer', summary: 'Not part of this milestone. The composer is the current one, visually unchanged, until its scope is revisited.' },
]

export type QuestionArg = 'rowAdaptive' | 'rosterLoading' | 'errorPlacement' | 'emptyMain' | 'paletteScope' | 'glyphRefinement' | 'sectionNav'

export const openQuestions: readonly OpenQuestion<QuestionArg>[] = [
  { id: 'RM1-Q1', arg: 'rowAdaptive', question: 'Below which sidebar width does SR-2 collapse to a one-line row? (The milestone sidebar opens at 304px; drag it to compare.)', options: [
    { value: '300', label: 'One line below 300px (metrics line from 300px)' },
    { value: '260', label: 'One line below 260px' },
    { value: 'off', label: 'Always two lines' },
  ] },
  { id: 'RM1-Q2', arg: 'rosterLoading', question: 'How does the roster look before the first agent list arrives?', options: [
    { value: 'skeleton', label: 'Row-shaped placeholders at real row height' },
    { value: 'line', label: 'One status line, no placeholders' },
  ] },
  { id: 'RM1-Q3', arg: 'errorPlacement', question: 'Where does a roster or connection failure appear?', options: [
    { value: 'sidebar', label: 'Inside the sidebar, where the list would be' },
    { value: 'banner', label: 'One strip across the top of the workspace' },
  ] },
  { id: 'RM1-Q4', arg: 'emptyMain', question: 'What fills the workspace when no agent is open?', options: [
    { value: 'hints', label: 'One line plus three keyboard hints' },
    { value: 'line', label: 'One line only' },
  ] },
  { id: 'RM1-Q5', arg: 'paletteScope', question: 'What does Mod+K search by default?', options: [
    { value: 'mixed', label: 'Agents and commands together' },
    { value: 'agents', label: 'Agents only; type ">" for commands' },
  ] },
  { id: 'RM1-Q6', arg: 'glyphRefinement', question: 'Which SG-1 refinement carries running and attention states?', options: [
    { value: 'G1', label: 'Plain discs' },
    { value: 'G2', label: 'Running disc gains a turning ring' },
    { value: 'G3', label: 'Non-running states add a shape cue' },
  ] },
  { id: 'RM1-Q7', arg: 'sectionNav', question: 'Where do Inbox, Missions and Resources live in the sidebar?', options: [
    { value: 'footer', label: 'One compact row under the agent list' },
    { value: 'header', label: 'One compact row above the agent list' },
  ] },
]
