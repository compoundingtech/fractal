import type { Grants } from '../data/source.ts'

/** What the paired device may do, in the words of its grants. Never carries the credential. */
export interface DeviceAccess {
  readonly level: 'read-only' | 'limited' | 'full'
  readonly title: string
  readonly allowed: readonly string[]
  readonly refused: readonly string[]
}

const capabilities: readonly (readonly [keyof Grants, string])[] = [
  ['messageSend', 'Send messages'],
  ['actions', 'Run agent actions'],
  ['attachmentUpload', 'Attach images'],
  ['terminalInput', 'Type in terminals'],
  ['terminalControl', 'Attach to terminals and resize them'],
]

const titles: Readonly<Record<DeviceAccess['level'], string>> = { 'read-only': 'Read-only', limited: 'Limited', full: 'Full control' }

/** Every paired device may observe; the grants add to it. */
const observe = 'View agents and terminal output'

export const deviceAccess = (grants: Grants): DeviceAccess => {
  const allowed = capabilities.filter(([grant]) => grants[grant] === 'granted').map(([, label]) => label)
  const refused = capabilities.filter(([grant]) => grants[grant] !== 'granted').map(([, label]) => label)
  const level = allowed.length === 0 ? 'read-only' : refused.length === 0 ? 'full' : 'limited'
  return { level, title: titles[level], allowed: [observe, ...allowed], refused }
}

/** Build facts injected by the app: Storybook and the embed have no `virtual:build-identity`. */
export interface SettingsBuild {
  readonly version: string
  readonly revision?: string
  readonly dirty?: boolean
}
