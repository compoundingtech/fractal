import { lineage, ranIn, stateAt, type MissionDetailModel, type StepState } from './detailModel.ts'
import type { MissionFocus } from './focus.ts'

// What the linked views agree on: which steps to dim and each step's state at the replay cursor.

/** Steps to dim: outside the hovered step's lineage, or not run inside the brushed range. */
export const dimmedSteps = (model: MissionDetailModel, focus: MissionFocus): ReadonlySet<string> => {
  const out = new Set<string>()
  const lit = focus.hovered === undefined ? undefined : lineage(model.steps, focus.hovered)
  for (const step of model.steps) {
    if (lit !== undefined && !lit.has(step.id)) out.add(step.id)
    if (focus.brush !== undefined && !ranIn(step, focus.brush[0], focus.brush[1], model.now)) out.add(step.id)
  }
  return out
}

/** What the views draw for a step: its state now, or at the replay cursor. */
export const shownState = (model: MissionDetailModel, id: string, cursor: number | undefined): StepState => {
  const step = model.steps.find(each => each.id === id)
  if (step === undefined) return 'pending'
  return cursor === undefined ? step.state : stateAt(step, cursor, model.now)
}

/** The step the inspector shows: the hovered one, else the picked one, else the one most worth a look. */
export const inspectedStep = (model: MissionDetailModel, focus: MissionFocus): { readonly id: string | undefined; readonly source: 'hovered' | 'selected' | 'default' } =>
  focus.hovered !== undefined && model.steps.some(step => step.id === focus.hovered) ? { id: focus.hovered, source: 'hovered' }
    : focus.selected !== undefined && model.steps.some(step => step.id === focus.selected) ? { id: focus.selected, source: 'selected' }
      : { id: defaultStep(model), source: 'default' }

/** Needs you, else failed, else running, else next, else the first step. */
export const defaultStep = (model: MissionDetailModel): string | undefined =>
  model.needsYou[0] ?? model.failed[0] ?? model.running[0] ?? model.next ?? model.steps[0]?.id

/** The selection as the views mark it: the picked step, else the default one. */
export const selectedStep = (model: MissionDetailModel, focus: MissionFocus): string | undefined =>
  focus.selected !== undefined && model.steps.some(step => step.id === focus.selected) ? focus.selected : defaultStep(model)
