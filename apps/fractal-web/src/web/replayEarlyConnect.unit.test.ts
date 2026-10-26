// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest'
import { fractalDevReplay, suppressReplayEarlyConnect } from '../../scripts/vite-replay.mjs'
import html from './index.html?raw'

const runEarlyConnect = (page: string, search: string) => {
  const script = page.match(/<script data-wf-early-connect>([\s\S]*?)<\/script>/)?.[1]
  if (script === undefined) throw new Error('Missing early-connect script')
  const WebSocket = vi.fn(() => { throw new Error('Stop after attempted real connection') })
  new Function('location', 'WebSocket', script)({ search, href: `http://localhost/${search}` }, WebSocket)
  return WebSocket
}
afterEach(() => vi.restoreAllMocks())

it.each(['first-run-onboarding', 'unknown', ''])('does not open any real socket for dev scenario %j', (id) => {
  expect(runEarlyConnect(suppressReplayEarlyConnect(html), `?scenario=${id}`)).not.toHaveBeenCalled()
})
it('keeps live startup the default in dev', () => {
  expect(runEarlyConnect(suppressReplayEarlyConnect(html), '')).toHaveBeenCalledOnce()
})
it('only applies the HTML hook in dev; production ignores the scenario query', () => {
  expect(fractalDevReplay().apply).toBe('serve')
  expect(runEarlyConnect(html, '?scenario=first-run-onboarding')).toHaveBeenCalledOnce()
})
