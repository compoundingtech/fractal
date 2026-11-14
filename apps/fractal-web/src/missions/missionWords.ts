import words from '../../../../fixtures/clients/words.json' with { type: 'json' }
import type { MissionPresentation } from './presentation.ts'

// The contract's mission vocabulary. Kept apart from `presentation.ts`, which the Missions tab marker
// loads with the shell: the JSON word list stays with the on-demand missions surfaces.

export type MissionWord = (typeof words.mission)[number]['id']

export const MISSION_WORDS: readonly MissionWord[] = words.mission.map((word) => word.id)
export const missionWord = (id: MissionWord) => words.mission.find((word) => word.id === id)!

/** Missions in `words.json` order, Fractal's order within each word. */
export const sortMissions = <T extends { readonly presentation: MissionPresentation }>(rows: readonly T[]): readonly T[] =>
  rows.map((row, index) => ({ row, index }))
    .sort((a, b) => MISSION_WORDS.indexOf(a.row.presentation.word) - MISSION_WORDS.indexOf(b.row.presentation.word) || a.index - b.index)
    .map(({ row }) => row)
