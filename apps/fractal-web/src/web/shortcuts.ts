import { detectPlatform, formatKeybinding, parseKeybinding, type Platform } from '../workbench-kit/keys.tsx'

/** One action's key input: alternatives, each a sequence of `parseKeybinding` chords. */
export interface ShortcutDefinition {
  readonly action: string
  readonly chords: readonly (readonly string[])[]
  /** Only exists where the devbar is mounted. */
  readonly devOnly?: boolean
}

/**
 * The shortcut reference: `?` help and settings render this table, and the sidebar's own handlers read their keys from it.
 * `focusComposer`, `focusSearch` and `toggleDevBar` keep their handlers in Composer, Toolbar and DevBar, so those three can still drift.
 */
export const shortcuts = {
  showAgents: { action: 'Show Agents', chords: [['g', 'a']] },
  showAlerts: { action: 'Show Alerts', chords: [['g', 'l']] },
  showMissions: { action: 'Show Missions', chords: [['g', 'm']] },
  switchSidebarTab: { action: 'Switch focused sidebar tabs', chords: [['left'], ['right']] },
  toggleSidebar: { action: 'Toggle sidebar', chords: [['mod+b']] },
  focusComposer: { action: 'Focus the message box', chords: [['mod+l']] },
  focusSearch: { action: 'Focus sidebar search', chords: [['/']] },
  showShortcuts: { action: 'Show shortcuts', chords: [['?']] },
  toggleDevBar: { action: 'Toggle devbar', chords: [['mod+shift+b']], devOnly: true },
} as const satisfies Record<string, ShortcutDefinition>

export const shortcutList = (devBar: boolean): readonly ShortcutDefinition[] =>
  Object.values<ShortcutDefinition>(shortcuts).filter(shortcut => devBar || shortcut.devOnly !== true)

/** Alternatives of steps of key caps: `[[['g'], ['a']]]`, or `[[['⌘', 'B']]]` on a Mac. */
export const shortcutCaps = (shortcut: ShortcutDefinition, platform: Platform = detectPlatform()): readonly (readonly (readonly string[])[])[] =>
  shortcut.chords.map(chord => chord.map(step => {
    const caps = formatKeybinding({ binding: parseKeybinding(step), platform })
    // A bare letter is typed as written: `g a`, while a modified one reads as its cap.
    return caps.length === 1 ? [step.length === 1 ? step : caps[0]!] : caps
  }))

/** The same caps as one line of text: `g a`, `← / →`, `⌘B`, `Ctrl+B`. */
export const shortcutText = (shortcut: ShortcutDefinition, platform: Platform = detectPlatform()): string =>
  shortcutCaps(shortcut, platform).map(chord => chord.map(caps => caps.join(platform === 'mac' ? '' : '+')).join(' ')).join(' / ')

/** A dialog carrying this keeps the shell's global chords from acting behind it. */
export const shortcutsPausedSelector = '[data-shortcuts-paused]'
