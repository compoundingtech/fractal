// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent, within } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { worldNow } from '../fixtures/world.ts'
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

it('sends each change with the chosen end and shows what Fractal answered', async () => {
  const port = fixtureAvailabilityPort('available')
  const set = vi.fn(port.set)
  const setNote = vi.fn(port.setNote)
  const dialog = within(await open({ ...port, set, setNote }))
  await userEvent.click(dialog.getByRole('radio', { name: '1 hour' }))
  await userEvent.click(dialog.getByRole('button', { name: 'Away' }))
  expect(set).toHaveBeenLastCalledWith({ status: { value: 'away', until: new Date(worldNow + 3_600_000).toISOString() } })
  expect(dialog.getByTestId('availability-facts').textContent).toContain('Away · until')
  // Following presence and the defaults have no end, whatever the chosen bound.
  await userEvent.click(dialog.getByRole('button', { name: 'Follow presence' }))
  await userEvent.click(dialog.getByRole('button', { name: 'All asks' }))
  expect(set.mock.calls.slice(-2)).toEqual([[{ status: { value: null } }], [{ interruptions: { value: 'all' } }]])
  await userEvent.click(dialog.getByRole('radio', { name: 'Tomorrow 9:00' }))
  await userEvent.type(dialog.getByRole('textbox', { name: 'Note' }), 'Back after lunch.')
  await userEvent.click(dialog.getByRole('button', { name: 'Save note' }))
  const morning = new Date(worldNow)
  expect(setNote).toHaveBeenLastCalledWith({ text: 'Back after lunch.', expires_at: new Date(morning.getFullYear(), morning.getMonth(), morning.getDate() + 1, 9).toISOString() })
  expect(dialog.getByTestId('availability-facts').textContent).toContain('Back after lunch.')
  await userEvent.click(dialog.getByRole('button', { name: 'Clear note' }))
  expect(setNote).toHaveBeenLastCalledWith({ text: null })
  expect(dialog.getByTestId('availability-facts').textContent).toContain('No note')
})

it('refuses a past custom end locally and shows a refused write in words', async () => {
  const set = vi.fn(async () => ({ _tag: 'Refused', detail: "This device's pairing does not allow changing your availability. Pair it again to change it here." }) as const)
  const dialog = within(await open({ set }))
  await userEvent.click(dialog.getByRole('radio', { name: 'Custom' }))
  await userEvent.type(dialog.getByLabelText('Until'), '2000-01-01T09:00')
  await userEvent.click(dialog.getByRole('button', { name: 'Away' }))
  expect(set).not.toHaveBeenCalled()
  expect(dialog.getByRole('alert').textContent).toBe('Choose a time in the future.')
  await userEvent.click(dialog.getByRole('button', { name: 'Hold all' }))
  expect(set).not.toHaveBeenCalled()
  await userEvent.click(dialog.getByRole('button', { name: 'Run' }))
  expect(set).toHaveBeenCalledWith({ deadline_clock: { value: 'run' } })
  expect(dialog.getByRole('alert').textContent).toContain('Pair it again to change it here.')
  expect(dialog.getByTestId('availability-facts').textContent).toContain('Running')
})
