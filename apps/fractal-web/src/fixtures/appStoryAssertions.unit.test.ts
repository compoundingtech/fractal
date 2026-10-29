// @vitest-environment jsdom
import { expect, it } from 'vitest'
import { assertProductionAppState } from './appStoryAssertions.ts'
import { appStoryStates } from './appStoryStates.ts'

it('negative control: the exact play assertion rejects a production component replaced by a stub', () => {
  const canvas = document.createElement('div')
  // Keep every state marker: matching the harness alone must not pass a play.
  for (const state of appStoryStates) {
    canvas.innerHTML = `<div data-app-state="${state}"><div>Stubbed production module</div></div>`
    expect(() => assertProductionAppState(canvas, state, false)).toThrow(`Production transcript missing in ${state}`)
    expect(() => assertProductionAppState(canvas, state, true)).toThrow(`Production transcript missing in ${state}`)
  }
})

it('negative control: a pane without the workspace fails the workspace play', () => {
  const canvas = document.createElement('div')
  canvas.innerHTML = '<div data-app-state="populated"><div data-testid="transcript-scroll"><div data-testid="transcript-turn"></div></div><div data-testid="conversation-composer-dock"></div></div>'
  expect(() => assertProductionAppState(canvas, 'populated', false)).not.toThrow()
  expect(() => assertProductionAppState(canvas, 'populated', true)).toThrow('Production workspace missing in populated')
})

it('negative control: rejects a gallery with multiple full production boundaries', () => {
  const canvas = document.createElement('div')
  canvas.innerHTML = '<div data-app-state="populated"></div><div data-app-state="loading"></div>'
  expect(() => assertProductionAppState(canvas, 'populated', false)).toThrow('A story must mount exactly one production app state')
  expect(() => assertProductionAppState(canvas, 'populated', true)).toThrow('A story must mount exactly one production app state')
})

it.each([
  { state: 'offline', marker: '<div data-connection-state="offline">Offline</div>', error: 'Production offline banner missing', workspace: true },
  { state: 'failed-tool', marker: '<div data-error-overlay-id="work-failure-tool"></div>', error: 'Production failed-tool error missing', workspace: false },
  { state: 'scripted-repair', marker: '<div data-testid="live-work" aria-label="Response in progress"></div>', error: 'Production streaming repair activity missing', workspace: false },
] as const)('requires the production $state witness, not merely a populated transcript', ({ state, marker, error, workspace }) => {
  const canvas = document.createElement('div')
  canvas.innerHTML = `<div data-app-state="${state}"><div data-testid="live-agent-workspace"></div><div data-testid="transcript-scroll"><div data-testid="transcript-turn"></div></div><div data-testid="conversation-composer-dock"></div></div>`
  expect(() => assertProductionAppState(canvas, state, workspace)).toThrow(error)
  canvas.firstElementChild!.insertAdjacentHTML('beforeend', marker)
  expect(() => assertProductionAppState(canvas, state, workspace)).not.toThrow()
})
