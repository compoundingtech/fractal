// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import { ClientControlIndicator, type ClientControlIndicatorState } from './ClientControlIndicator.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const host = document.createElement('div')
const root = createRoot(host)
afterEach(async () => { await act(async () => root.render(null)); vi.unstubAllGlobals() })

const render = async (state: ClientControlIndicatorState, onRevokeEnrollment = () => {}) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  await act(async () => root.render(<ClientControlIndicator state={state} onRevokeEnrollment={onRevokeEnrollment} />))
}

it('renders nothing when no grant is active', async () => {
  await render({ _tag: 'Inactive' })
  expect(host.childElementCount).toBe(0)
})

it('names the run and invokes enrollment revoke rather than hiding the grant optimistically', async () => {
  let revokes = 0
  await render({ _tag: 'Active', runId: 'mission-run/fixture' }, () => { revokes += 1 })
  expect(host.querySelector('[data-client-control-run]')?.getAttribute('data-client-control-run')).toBe('mission-run/fixture')
  expect(host.querySelector('code')?.textContent).toBe('mission-run/fixture')
  const button = host.querySelector('button')!
  expect(button.textContent).toBe('Revoke test enrollment')
  await act(async () => button.click())
  expect(revokes).toBe(1)
  expect(host.querySelector('[aria-label="Test client control"]')).not.toBeNull()
})

it('keeps the run visible during revocation and explains an unconfirmed revoke', async () => {
  await render({ _tag: 'Revoking', runId: 'mission-run/fixture' })
  expect(host.querySelector('button')?.disabled).toBe(true)
  expect(host.querySelector('code')?.textContent).toBe('mission-run/fixture')
  await render({ _tag: 'RevokeFailed', runId: 'mission-run/fixture' })
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('not confirmed')
  expect(host.querySelector('button')?.disabled).toBe(false)
})
