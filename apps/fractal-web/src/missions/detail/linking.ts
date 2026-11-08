import type { MissionDetailModel, StepState } from './detailModel.ts'
import type { MissionFocus } from './focus.ts'

// What the linked views agree on. Not yet implemented: the tests in linking.unit.test.ts pin it first.

export const dimmedSteps = (_model: MissionDetailModel, _focus: MissionFocus): ReadonlySet<string> => new Set()
export const shownState = (_model: MissionDetailModel, _id: string, _cursor: number | undefined): StepState => 'pending'
export const inspectedStep = (_model: MissionDetailModel, _focus: MissionFocus): { readonly id: string | undefined; readonly source: 'hovered' | 'selected' | 'default' } => ({ id: undefined, source: 'default' })
export const defaultStep = (_model: MissionDetailModel): string | undefined => undefined
export const selectedStep = (_model: MissionDetailModel, _focus: MissionFocus): string | undefined => undefined
