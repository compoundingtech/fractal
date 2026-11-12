import { describe, expect, it, vi } from 'vitest'
import type { Grants } from '../data/source.ts'
import { deviceAccess } from './settingsModel.ts'
import { shortcutCaps, shortcutList, shortcuts, shortcutText } from './shortcuts.ts'

// The chord formatter shares a module with its chips; Node cannot compile their StyleX.
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

const none: Grants = { actions: 'ungranted', messageSend: 'ungranted', attachmentUpload: 'ungranted', terminalInput: 'ungranted', terminalControl: 'ungranted' }

describe('deviceAccess', () => {
  it('names a device with no grants read-only', () => {
    expect(deviceAccess(none)).toMatchObject({ level: 'read-only', title: 'Read-only', allowed: ['View agents and terminal output'] })
  })
  it('names a device with every grant full control', () => {
    const all: Grants = { actions: 'granted', messageSend: 'granted', attachmentUpload: 'granted', terminalInput: 'granted', terminalControl: 'granted' }
    expect(deviceAccess(all)).toMatchObject({ level: 'full', title: 'Full control', refused: [] })
  })
  it('lists both sides of a partial grant', () => {
    const access = deviceAccess({ ...none, messageSend: 'granted' })
    expect(access.level).toBe('limited')
    expect(access.allowed).toEqual(['View agents and terminal output', 'Send messages'])
    expect(access.refused).toContain('Type in terminals')
  })
})

describe('shortcut reference', () => {
  it('formats one platform at a time', () => {
    expect(shortcutText(shortcuts.toggleSidebar, 'mac')).toBe('⌘B')
    expect(shortcutText(shortcuts.toggleSidebar, 'other')).toBe('Ctrl+B')
    expect(shortcutText(shortcuts.showAgents, 'mac')).toBe('g a')
    expect(shortcutText(shortcuts.switchSidebarTab, 'other')).toBe('← / →')
    expect(shortcutCaps(shortcuts.toggleDevBar, 'other')).toEqual([[['Ctrl', 'Shift', 'B']]])
  })
  it('hides the devbar chord where no devbar exists', () => {
    expect(shortcutList(false)).not.toContain(shortcuts.toggleDevBar)
    expect(shortcutList(true)).toContain(shortcuts.toggleDevBar)
  })
})
