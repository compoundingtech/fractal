// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import type { Grants } from '../data/source.ts'
import { SettingsControl, type SettingsScheme } from './SettingsDialog.tsx'

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

const none: Grants = { actions: 'ungranted', messageSend: 'ungranted', attachmentUpload: 'ungranted', terminalInput: 'ungranted', terminalControl: 'ungranted' }
const mount = async (onSchemeChange: (scheme: SettingsScheme) => void = () => undefined) => {
  const source = { ...fixtureSource({ world: fixtureProjections }), gateway: 'gateway.example', grants: Atom.make(none) }
  await act(async () => root.render(<DataSourceProvider source={source} registry={AtomRegistry.make()}>
    <SettingsControl scheme="dark" onSchemeChange={onSchemeChange} build={{ version: '1.2.3', revision: 'abcdef1234567', dirty: true }} />
  </DataSourceProvider>))
  const gear = within(host).getByRole('button', { name: 'Settings' })
  gear.focus()
  await act(async () => gear.click())
  return { gear, dialog: within(document.body).getByRole('dialog', { name: 'Settings' }) }
}

it('reports only what the client holds about this device, and no credential', async () => {
  const { dialog } = await mount()
  const facts = within(dialog).getByRole('region', { name: 'Device and connection' }).textContent
  expect(facts).toContain('Read-only')
  expect(facts).toContain('gateway.example')
  expect(facts).toContain('1.2.3 · abcdef123 · dirty')
  expect(facts).not.toMatch(/bearer/i)
})

it('moves theme switching here as a named Light, Dark or System choice', async () => {
  const change = vi.fn()
  const { dialog } = await mount(change)
  const group = within(dialog).getByRole('radiogroup', { name: 'Theme' })
  expect(within(group).getByRole<HTMLInputElement>('radio', { name: 'Dark' }).checked).toBe(true)
  await act(async () => within(group).getByRole('radio', { name: 'Light' }).click())
  expect(change).toHaveBeenCalledWith('light')
  await act(async () => within(group).getByRole('radio', { name: 'System' }).click())
  expect(change).toHaveBeenCalledWith('system')
})

it('lists the shortcut table and the devbar switch, and carries no usage entry', async () => {
  const { dialog } = await mount()
  expect(within(dialog).getByRole('region', { name: 'Keyboard shortcuts' }).textContent).toContain('Toggle sidebar')
  expect(within(dialog).getByRole('switch', { name: 'Show devbar' })).toBeTruthy()
  expect(within(dialog).getByRole('switch', { name: 'Show all system events' })).toBeTruthy()
  expect(within(dialog).queryByRole('button', { name: /usage/i })).toBeNull()
})

it('closes on Escape and returns focus to the gear', async () => {
  const { gear, dialog } = await mount()
  await act(async () => { dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); await new Promise(resolve => setTimeout(resolve, 50)) })
  expect(within(document.body).queryByRole('dialog', { name: 'Settings' })).toBeNull()
  expect(document.activeElement).toBe(gear)
})
