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
import type { BrowserPairingPort, Grants } from '../data/source.ts'
import { BrowserPairingControl } from './BrowserPairing.tsx'
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
const mount = async (onSchemeChange: (scheme: SettingsScheme) => void = () => undefined, pairing?: BrowserPairingPort, grants: Grants = none) => {
  const source = { ...fixtureSource({ world: fixtureProjections }), gateway: 'gateway.example', grants: Atom.make(grants), ...(pairing === undefined ? {} : { pairing }) }
  await act(async () => root.render(<DataSourceProvider source={source} registry={AtomRegistry.make()}>
    <BrowserPairingControl />
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
  expect(facts).toContain('No controls granted')
  expect(facts).not.toContain('View agents and terminal output')
  expect(facts).toContain('gateway.example')
  expect(facts).toContain('1.2.3 · abcdef123 · dirty')
  expect(facts).not.toMatch(/bearer/i)
})

it('describes every listed grant without implying unrestricted access', async () => {
  const grants: Grants = { actions: 'granted', messageSend: 'granted', attachmentUpload: 'granted', terminalInput: 'granted', terminalControl: 'granted' }
  const { dialog } = await mount(() => undefined, undefined, grants)
  const facts = within(dialog).getByRole('region', { name: 'Device and connection' }).textContent
  expect(facts).toContain('All listed controls granted')
  expect(facts).not.toContain('Full control')
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

it('reuses pending browser access in Settings with one accessible code and restores the workspace control on close', async () => {
  const status = vi.fn(async () => ({ status: 'pending' as const, displayCode: 'PAIR-123' }))
  const pairing: BrowserPairingPort = { status, start: vi.fn(async () => ({ requestId: 'request/1', displayCode: 'PAIR-123' })), forget: vi.fn(async () => {}) }
  const { dialog, gear } = await mount(() => undefined, pairing)
  expect(document.querySelectorAll('[data-testid="pairing-code"]')).toHaveLength(2)
  const accessiblePairing = within(document.body).getByRole('region', { name: 'Browser pairing' })
  expect(dialog.contains(accessiblePairing)).toBe(true)
  expect(accessiblePairing.textContent).toContain('PAIR-123')
  expect(dialog.textContent).not.toContain('WF_ST_AUTHORIZATION_FILE')
  expect(status).toHaveBeenCalledOnce()
  await act(async () => within(dialog).getByRole('button', { name: 'Close settings' }).click())
  expect(document.querySelectorAll('[data-testid="pairing-code"]')).toHaveLength(1)
  expect(within(document.body).getByRole('region', { name: 'Browser pairing' }).textContent).toContain('PAIR-123')
  expect(status).toHaveBeenCalledOnce()
  expect(document.activeElement).toBe(gear)
})

it('keeps Forget in Settings for a paired browser with no control grants', async () => {
  const pairing: BrowserPairingPort = { status: vi.fn(async () => ({ status: 'paired' as const })), start: vi.fn(async () => ({ requestId: 'request/1', displayCode: 'PAIR-123' })), forget: vi.fn(async () => {}) }
  const { dialog } = await mount(() => undefined, pairing)
  expect(document.querySelectorAll('[data-testid="browser-pairing"]')).toHaveLength(1)
  expect(within(dialog).getByRole('button', { name: 'Forget this browser' })).toBeTruthy()
  expect(dialog.textContent).toContain('No controls granted')
  expect(dialog.textContent).not.toContain('Read-only')
})
