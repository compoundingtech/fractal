// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { SettingsControl } from '../web/SettingsDialog.tsx'
import type { AvailabilityPort, AvailabilityRead } from './availability-port.ts'
import { fixtureAvailabilityPort, observedAvailability } from './availability.fixtures.ts'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'animation',
  props: () => ({}),
}))

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

const open = async (availability: Partial<AvailabilityPort> | undefined) => {
  const source = { ...fixtureSource({ world: fixtureProjections }), ...(availability === undefined ? {} : { availability: { ...fixtureAvailabilityPort('available'), ...availability } }) }
  await act(async () => root.render(<DataSourceProvider source={source} registry={AtomRegistry.make()}>
    <SettingsControl scheme="dark" onSchemeChange={() => undefined} />
  </DataSourceProvider>))
  await act(async () => within(host).getByRole('button', { name: 'Settings' }).click())
  return within(document.body).getByRole('dialog', { name: 'Settings' })
}

it('reads availability when Settings opens and again when the page regains focus', async () => {
  const read = vi.fn(async (): Promise<AvailabilityRead> => observedAvailability('away'))
  const dialog = await open({ read })
  const facts = within(dialog).getByTestId('availability-facts')
  expect(read).toHaveBeenCalledTimes(1)
  expect(facts.textContent).toContain('Away · until')
  expect(facts.textContent).toContain('Set by you')
  expect(facts.textContent).toContain('Urgent asks only')
  expect(facts.textContent).toContain('Held for routine asks')
  expect(facts.textContent).toContain('Ship the release notes first.')
  read.mockResolvedValueOnce(observedAvailability('available'))
  await act(async () => window.dispatchEvent(new Event('focus')))
  expect(read).toHaveBeenCalledTimes(2)
  expect(facts.textContent).toContain('Follows your presence')
  expect(facts.textContent).toContain('No note')
})

it('explains a refused read in words, and shows no section without a gateway', async () => {
  const dialog = await open({ read: async () => ({ _tag: 'Unavailable', detail: 'Reading availability has not been granted to this device.' }) })
  expect(within(dialog).getByRole('status').textContent).toBe('Reading availability has not been granted to this device.')
  await act(async () => root.unmount())
  root = createRoot(host)
  const fixtures = await open(undefined)
  expect(within(fixtures).queryByRole('region', { name: 'Availability' })).toBeNull()
})
