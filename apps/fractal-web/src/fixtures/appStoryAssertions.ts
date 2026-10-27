import type { AppStoryState } from './appStoryStates.ts'

/** Used by both story plays and the planted stub control. A harness marker is never
 * evidence: the selected state must contain the real transcript and composer. */
export const assertProductionAppState = (canvas: HTMLElement, state: AppStoryState, workspace: boolean): void => {
  if (canvas.querySelectorAll('[data-app-state]').length !== 1)
    throw new Error('A story must mount exactly one production app state')
  const specimen = canvas.querySelector(`[data-app-state="${state}"]`)
  const transcript = state === 'unavailable' ? 'transcript-unavailable' : 'transcript-scroll'
  if (specimen === null || specimen.querySelector(`[data-testid="${transcript}"]`) === null)
    throw new Error(`Production transcript missing in ${state}`)
  if (specimen.querySelector('[data-testid="conversation-composer-dock"]') === null)
    throw new Error(`Production composer missing in ${state}`)
  if (workspace && specimen.querySelector('[data-testid="live-agent-workspace"]') === null)
    throw new Error(`Production workspace missing in ${state}`)
  if (state === 'empty' && specimen.querySelector('[data-testid="transcript-empty"]') === null)
    throw new Error('Production empty transcript missing')
  if (state === 'loading' && specimen.querySelector('[data-testid="transcript-placeholder"]') === null)
    throw new Error('Production loading transcript missing')
  if (['populated', 'failed-tool', 'offline', 'scripted-repair'].includes(state) && specimen.querySelector('[data-testid="transcript-turn"]') === null)
    throw new Error(`Production populated transcript missing in ${state}`)
  if (state === 'unavailable' && specimen.querySelector('[data-wf-unavailable]') === null)
    throw new Error('Production unavailable transcript missing')
  if (state === 'offline' && workspace && specimen.querySelector('[data-connection-state="offline"]') === null)
    throw new Error('Production offline banner missing')
  if (state === 'failed-tool' && specimen.querySelector('[data-error-overlay-id^="work-failure-"]') === null)
    throw new Error('Production failed-tool error missing')
  if (state === 'scripted-repair' && specimen.querySelector('[data-testid="live-work"][aria-label="Response in progress"]') === null)
    throw new Error('Production streaming repair activity missing')
}
