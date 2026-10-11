// @vitest-environment jsdom
import { expect, it } from 'vitest'
import { assertProductionAppState } from './appStoryAssertions.ts'
import { appStoryStates } from './storybookScenarios.ts'

it('negative control: the exact play assertion rejects a production component replaced by a stub', () => {
  const canvas = document.createElement('div')
  // Keep all story labels: matching the harness alone must not pass a play.
  for (const state of appStoryStates) {
    canvas.innerHTML = `<section data-app-state="${state}"><div>Stubbed production module</div></section>`
    expect(() => assertProductionAppState(canvas, state, false)).toThrow(`Production transcript missing in ${state}`)
    expect(() => assertProductionAppState(canvas, state, true)).toThrow(`Production transcript missing in ${state}`)
  }
  console.info('NEGATIVE CONTROL: production stubs rejected in all 7 states (pane and workspace plays)')
})

it('negative control: rejects a gallery with multiple full production boundaries', () => {
  const canvas = document.createElement('div')
  canvas.innerHTML = '<section data-app-state="populated"></section><section data-app-state="loading"></section>'
  expect(() => assertProductionAppState(canvas, 'populated', false)).toThrow('A story must mount exactly one production app state')
  expect(() => assertProductionAppState(canvas, 'populated', true)).toThrow('A story must mount exactly one production app state')
})
