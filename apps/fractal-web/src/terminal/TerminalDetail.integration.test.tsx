// @vitest-environment jsdom
import { Agent, decodeUnknownSync, type TerminalScreen } from '@smalltalk/st3-client/schema'
import type { PtyOutput, PtySession, PtySize, PtyState, PtyTarget } from '@st3/sdk/effect'
import { Effect, SubscriptionRef } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { observed, unavailable, waiting, type Feed } from '../data/source.ts'
import { makeScreen } from './fixtures.ts'
import type { RawTerminalPort } from './rawTerminalState.ts'
import { TerminalDetail } from './TerminalDetail.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

const ref = 'terminal/example'
// The daemon names the terminal `terminal/<agent id>`, not by the view's subject address.
const screen: TerminalScreen = { ...makeScreen({ scene: 'F3', columns: 40, rows: 4, frame: 0 }), terminal_id: 'terminal/agent/example' }
const other: `terminal/${string}` = 'terminal/other'
const otherScreen: TerminalScreen = { ...screen, terminal_id: 'terminal/agent/other', runtime_incarnation: 'incarnation/other' }
const agent = (reachability: 'reachable' | 'unreachable') => decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/example', runtime_ids: [], reachability, state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-04T12:00:00.000Z' })
const utf8 = new TextEncoder()
let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
let frames: FrameRequestCallback[]
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  frames = []
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback))
  vi.stubGlobal('cancelAnimationFrame', () => {})
  // jsdom fetches no module assets; serve the committed Ghostty module the browser would load.
  vi.stubGlobal('fetch', async () => new Response(new Uint8Array(await readFile(join(import.meta.dirname, 'assets/ghostty-key-encoder.generated.wasm'))), { headers: { 'content-type': 'application/wasm' } }))
  // jsdom has no font loading; the registered terminal faces resolve as loaded.
  Object.defineProperty(document, 'fonts', { configurable: true, value: ['Wf Terminal Mono', 'Wf Terminal Nerd Mono'].map((family) => ({ family, load: async () => [] })) })
  registry = AtomRegistry.make()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => {
  await act(async () => root.unmount())
  registry.dispose()
  host.remove()
  vi.unstubAllGlobals()
})
const isFeed = (terminal: Feed<TerminalScreen> | Readonly<Record<string, Feed<TerminalScreen>>>): terminal is Feed<TerminalScreen> =>
  typeof terminal._tag === 'string'
const mount = async ({ terminal, reachability = 'reachable', terminalRaw, at = ref }: {
  readonly terminal: Feed<TerminalScreen> | Readonly<Record<string, Feed<TerminalScreen>>>
  readonly reachability?: 'reachable' | 'unreachable'
  readonly terminalRaw?: RawTerminalPort
  readonly at?: `terminal/${string}`
}) => {
  const source = fixtureSource({ world: { now: 0, agents: [agent(reachability)], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } }, overrides: { terminal: isFeed(terminal) ? { [ref]: terminal } : terminal } })
  await act(async () => root.render(<DataSourceProvider source={terminalRaw === undefined ? source : { ...source, terminalRaw }} registry={registry}><React.Suspense fallback={<p>suspended</p>}><TerminalDetail key={at} address={{ ref: at, presentation: 'detail' }} visibility="visible" /></React.Suspense></DataSourceProvider>))
}
const chip = () => host.querySelector('[data-terminal-mode]')
const field = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="Terminal input"]')
const paint = () => act(async () => {
  for (const callback of frames.splice(0)) callback(0)
})

interface FakeSession {
  readonly target: PtyTarget
  readonly size: PtySize
  readonly sent: string[]
  readonly resized: PtySize[]
  readonly emit: (output: PtyOutput) => void
  readonly go: (state: PtyState) => Promise<void>
  closed: boolean
}

/** A raw port whose sessions the test drives: state transitions, output frames, and recorded DATA. */
const rawSessions = () => {
  const opened: FakeSession[] = []
  const decode = new TextDecoder()
  const port: RawTerminalPort = {
    open: ({ target, size, onOutput }) =>
      Effect.gen(function* () {
        const state = yield* SubscriptionRef.make<PtyState>({ _tag: 'Connecting', attempt: 1 })
        let current: PtyState = { _tag: 'Connecting', attempt: 1 }
        const entry: FakeSession = {
          target,
          size,
          sent: [],
          resized: [],
          emit: (output) => act(async () => onOutput(output)),
          go: (next) => act(async () => {
            current = next
            Effect.runSync(SubscriptionRef.set(state, next))
          }),
          closed: false,
        }
        opened.push(entry)
        yield* Effect.addFinalizer(() => Effect.sync(() => {
          entry.closed = true
        }))
        const session: PtySession = {
          state,
          send: (bytes) => {
            if (current._tag !== 'Live') return false
            entry.sent.push(decode.decode(bytes))
            return true
          },
          resize: (next) => {
            entry.resized.push(next)
          },
          resetInputModes: () => false,
        }
        return session
      }),
  }
  return { opened, port }
}

/** Attach, go live and paint the server's SCREEN replay. */
const live = async (session: FakeSession, text: string) => {
  await session.go({ _tag: 'Live', rows: 4, cols: 40 })
  await session.emit({ _tag: 'Geometry', rows: 4, cols: 40 })
  await session.emit({ _tag: 'Screen', bytes: utf8.encode(text) })
  await paint()
}

describe('terminal detail', () => {
  it.each([
    ['loading', waiting, 'Loading terminal…'],
    ['ungranted', unavailable({ reason: 'ungranted', detail: 'Terminal viewing is not permitted.' }), 'Terminal viewing is not permitted.'],
    ['unsupported', unavailable({ reason: 'unsupported', detail: 'This gateway does not provide terminals.' }), 'This gateway does not provide terminals.'],
    ['failed', unavailable({ reason: 'failed', detail: 'This agent has no terminal.', code: 'no-terminal' }), 'This agent has no terminal.'],
  ] as const)('states %s with its specific reason', async (state, terminal, copy) => {
    await mount({ terminal })
    const status = host.querySelector('[data-terminal-state]')
    expect(status?.getAttribute('data-terminal-state')).toBe(state)
    expect(host.textContent).toBe(copy)
    expect(host.textContent).not.toMatch(/\b(undefined|unknown)\b/i)
  })

  it.each(['', '   ', 'undefined', 'unknown'])('does not render a missing reason (%s)', async detail => {
    await mount({ terminal: unavailable({ reason: 'failed', detail }) })
    expect(host.textContent).toBe('The terminal could not be opened.')
    expect(host.textContent).not.toMatch(/\b(undefined|unknown)\b/i)
  })

  it('renders the projected grid read-only when the source has no raw stream, with history honestly unavailable', async () => {
    await mount({ terminal: observed({ value: screen }) })
    const grid = host.querySelector(`[aria-label="Terminal: ${screen.title}"]`)
    expect(grid?.getAttribute('data-terminal-renderer')).toBe('T1')
    expect(grid?.querySelectorAll('[data-terminal-row]')).toHaveLength(screen.lines.length)
    expect(grid?.textContent).toContain(screen.lines.find((line) => line.text.trim() !== '')?.text.trim())
    expect(host.querySelector('[aria-label="Live"]')).not.toBeNull()
    expect(chip()?.textContent).toBe('Read-only')
    expect(host.textContent).toContain('This gateway does not offer a live terminal stream.')
    expect(field()).toBeNull()
    expect(host.textContent).not.toContain('Enable input')
    expect(host.textContent).not.toContain('Terminal keyboard')
    expect(host.textContent).toContain('does not publish terminal history yet')
  })

  it('marks the terminal ended when its host is unreachable', async () => {
    await mount({ terminal: observed({ value: screen }), reachability: 'unreachable' })
    expect(host.querySelector('[aria-label="Ended"]')).not.toBeNull()
  })

  it('keeps the last grid but says it is stale', async () => {
    await mount({ terminal: observed({ value: screen, freshness: 'stale' }) })
    expect(host.textContent).toContain('Terminal snapshot is stale.')
    expect(host.querySelector(`[aria-label="Terminal: ${screen.title}"]`)).not.toBeNull()
    expect(host.querySelector('[aria-label="Live"]')).toBeNull()
    expect(host.querySelector('[aria-label="Stale"]')).not.toBeNull()
  })
})

describe('raw terminal', () => {
  it('attaches to the shown incarnation, paints the replay through libghostty, and types on the same socket', async () => {
    const { opened, port } = rawSessions()
    await mount({ terminal: observed({ value: screen }), terminalRaw: port })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    const session = opened[0]!
    expect(session.target).toEqual({ terminalId: 'terminal/agent/example', incarnation: screen.runtime_incarnation })
    expect(session.size).toEqual({ cols: 40, rows: 4 })
    // Until the replay paints, the projected screen stands in.
    expect(chip()?.textContent).toBe('Connecting…')
    await live(session, 'raw \x1b[1mprompt\x1b[0m$ ')
    expect(host.querySelector('[aria-roledescription="terminal"]')?.textContent).toContain('raw prompt$')
    expect(chip()?.textContent).toBe('Click to type')
    expect(host.querySelector('[aria-label="Live"]')).not.toBeNull()
    await act(async () => field()!.focus())
    expect(chip()?.textContent).toBe('Typing')
    await act(async () => {
      field()!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'l', code: 'KeyL' }))
      field()!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'c', code: 'KeyC', ctrlKey: true }))
      field()!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'Enter', code: 'Enter' }))
    })
    expect(session.sent).toEqual(['l', '\x03', '\r'])
    // Echo arrives as DATA and is painted on the next frame.
    await session.emit({ _tag: 'Data', bytes: utf8.encode('l\r\n') })
    await paint()
    expect(host.querySelector('[aria-roledescription="terminal"]')?.textContent).toContain('raw prompt$ l')
  })

  it('returns the emulator viewport from scrollback to live output with Latest, without typing', async () => {
    const { opened, port } = rawSessions()
    await mount({ terminal: observed({ value: screen }), terminalRaw: port })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    const session = opened[0]!
    await live(session, Array.from({ length: 12 }, (_, line) => `line${line}`).join('\r\n'))
    const grid = () => host.querySelector('[aria-roledescription="terminal"]')?.textContent ?? ''
    expect(grid()).toContain('line11')
    await act(async () => {
      host.querySelector('[aria-roledescription="terminal"]')!.dispatchEvent(new WheelEvent('wheel', { bubbles: true, deltaMode: 1, deltaY: -6 }))
    })
    await paint()
    expect(grid()).not.toContain('line11')
    const latest = [...host.querySelectorAll('button')].find((button) => button.textContent === 'Latest')!
    await act(async () => latest.click())
    await paint()
    expect(grid()).toContain('line11')
    expect(session.sent).toEqual([])
    expect(chip()?.textContent).toBe('Click to type')
    await session.emit({ _tag: 'Data', bytes: utf8.encode('\r\nline12') })
    await paint()
    expect(grid()).toContain('line12')
  })

  it('hands keyboard focus to the cursor field when the grid is tabbed to, so an IME composes from the first key', async () => {
    const { opened, port } = rawSessions()
    await mount({ terminal: observed({ value: screen }), terminalRaw: port })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    const session = opened[0]!
    await live(session, 'raw prompt$ ')
    await act(async () => host.querySelector<HTMLElement>('[aria-roledescription="terminal"]')!.focus())
    expect(document.activeElement).toBe(field())
    expect(chip()?.textContent).toBe('Typing')
    await act(async () => {
      field()!.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true, data: '' }))
      field()!.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '日本' }))
    })
    expect(session.sent).toEqual(['日本'])
    // Ctrl-\ leaves typing and parks focus on the grid instead of bouncing back into the field.
    await act(async () => {
      field()!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: '\\', code: 'Backslash', ctrlKey: true }))
    })
    expect(document.activeElement?.getAttribute('aria-roledescription')).toBe('terminal')
    expect(chip()?.textContent).toBe('Click to type')
  })

  it('drops keys while reconnecting instead of queueing them, and says so', async () => {
    const { opened, port } = rawSessions()
    await mount({ terminal: observed({ value: screen }), terminalRaw: port })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    const session = opened[0]!
    await live(session, '$ ')
    await session.go({ _tag: 'Reconnecting', attempt: 1, delayMs: 500, issue: 'PTY socket closed' })
    expect(chip()?.textContent).toBe('Reconnecting…')
    await act(async () => {
      field()!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'x', code: 'KeyX' }))
    })
    expect(session.sent).toEqual([])
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('The terminal is not connected. Nothing was sent, and keys are never replayed.')
    await session.go({ _tag: 'Live', rows: 4, cols: 40 })
    expect(session.sent).toEqual([])
  })

  it('falls back to the read-only projected screen with a clear chip when ATTACH is refused', async () => {
    const { opened, port } = rawSessions()
    await mount({ terminal: observed({ value: screen }), terminalRaw: port })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    await opened[0]!.go({ _tag: 'Refused', message: 'forbidden', status: 403, code: 'forbidden' })
    expect(chip()?.textContent).toBe('Read-only')
    expect(chip()?.getAttribute('data-terminal-mode')).toBe('ReadOnly')
    expect(host.textContent).toContain('This device may watch this terminal but not type into it.')
    expect(host.querySelector(`[aria-label="Terminal: ${screen.title}"]`)?.querySelectorAll('[data-terminal-row]')).toHaveLength(screen.lines.length)
    expect(field()).toBeNull()
  })

  it('detaches when the pane leaves and attaches the next terminal fresh', async () => {
    const { opened, port } = rawSessions()
    const terminals = { [ref]: observed({ value: screen }), [other]: observed({ value: otherScreen }) }
    await mount({ terminal: terminals, terminalRaw: port })
    await vi.waitFor(() => expect(opened).toHaveLength(1))
    await live(opened[0]!, '$ ')
    await mount({ terminal: terminals, terminalRaw: port, at: other })
    await vi.waitFor(() => expect(opened).toHaveLength(2))
    expect(opened[0]!.closed).toBe(true)
    expect(opened[1]!.target).toEqual({ terminalId: otherScreen.terminal_id, incarnation: otherScreen.runtime_incarnation })
    expect(opened[1]!.sent).toEqual([])
  })
})
