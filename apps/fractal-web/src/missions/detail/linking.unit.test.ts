import { describe, expect, it } from 'vitest'

import { lineage, missionDetailModel } from './detailModel.ts'
import { syncAttention, syncMission, syncNow, syncStart, syncStep, syncUsageRows, withServedHistory } from './fixtures.ts'
import { defaultStep, dimmedSteps, inspectedStep, selectedStep, shownState } from './linking.ts'
import { traceLine } from './traceLine.ts'

const MIN = 60_000
const HOUR = 60 * MIN
const at = (h: number, m = 0) => syncStart + h * HOUR + m * MIN
const model = missionDetailModel({ mission: withServedHistory(syncMission), agents: [], attention: syncAttention, usage: syncUsageRows, now: syncNow })
const live = model.steps.filter(step => step.state !== 'cancelled')
const find = (path: string) => model.steps.find(step => step.path === path)!

describe('linked views: what dims', () => {
  it('dims everything outside the hovered step\'s lineage', () => {
    const dim = dimmedSteps(model, { hovered: syncStep('docs') })
    const lit = lineage(model.steps, syncStep('docs'))
    expect(lit.has(syncStep('sdk'))).toBe(true)
    for (const step of live) expect(dim.has(step.id)).toBe(!lit.has(step.id))
  })
  it('dims steps that did not run inside the brushed range', () => {
    const dim = dimmedSteps(model, { brush: [at(0), at(1)] })
    expect(dim.has(syncStep('spec'))).toBe(false)
    expect(dim.has(syncStep('web'))).toBe(true)
  })
  it('dims nothing without hover or brush', () => {
    expect(dimmedSteps(model, {}).size).toBe(0)
  })
})

describe('linked views: what the inspector shows', () => {
  it('follows the hovered step, else the picked one, else the one most worth a look', () => {
    expect(inspectedStep(model, { hovered: syncStep('docs'), selected: syncStep('sdk') })).toEqual({ id: syncStep('docs'), source: 'hovered' })
    expect(inspectedStep(model, { selected: syncStep('sdk') })).toEqual({ id: syncStep('sdk'), source: 'selected' })
    // The security review waits on the person: that comes first.
    expect(inspectedStep(model, {})).toEqual({ id: syncStep('security'), source: 'default' })
    expect(defaultStep(model)).toBe(syncStep('security'))
  })
  it('ignores a hover or pick that names no step of this run', () => {
    expect(inspectedStep(model, { hovered: 'step-run/gone', selected: 'step-run/gone' }).source).toBe('default')
    expect(selectedStep(model, { selected: 'step-run/gone' })).toBe(syncStep('security'))
  })
  it('shows each step as it was at the replay cursor', () => {
    expect(shownState(model, syncStep('server'), at(3))).toBe('running')
    expect(shownState(model, syncStep('server'), at(9))).toBe('done')
    expect(shownState(model, syncStep('server'), undefined)).toBe('done')
    expect(shownState(model, syncStep('web'), at(9))).toBe('pending')
  })
})

describe('trace row second line: who and what it cost', () => {
  it('names the agent, how long it ran, its spend and its tries', () => {
    const line = traceLine(find('server'), 'workbench-shell', syncNow)
    expect(line.parts).toEqual(['workbench-shell', 'ran 5h 30m', '$11', '3 tries'])
    expect(line.you).toBeUndefined()
  })
  it('adds the wait on the person only when there was one', () => {
    expect(traceLine(find('security'), 'missions-ui', syncNow).you).toBe('waited on you 5h')
  })
  it('says a step has not started, and falls back to its owner', () => {
    const line = traceLine(find('canary'), undefined, syncNow)
    expect(line.parts).toEqual(['nobody yet', 'not started'])
  })
})
