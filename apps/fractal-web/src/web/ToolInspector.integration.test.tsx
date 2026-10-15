// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import type { WorkLogCall } from '@smalltalk/fractal-ui/assistant-ui/work-log'
import { DataSourceProvider } from '../data/react.tsx'
import { createToolInspectorFixture, inspectorFixtureRef, inspectorItems } from '../data/toolInspectorFixture.ts'
import { ConversationPane } from './ConversationPane.tsx'
import { LiveToolInspector, ToolInspector, ToolInspectorContent } from './ToolInspector.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))
const Host = () => {
  const [call, setCall] = React.useState<WorkLogCall>()
  return <><ConversationPane agentRef={inspectorFixtureRef} agentName="Row review" onOpenTool={setCall} />{call !== undefined && <LiveToolInspector agentRef={inspectorFixtureRef} call={call} onClose={() => setCall(undefined)} />}</>
}

it('opens real run/read/change/raw inspectors from transcript calls and reflects a running result as it arrives', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  const fixture = createToolInspectorFixture()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => root.render(<DataSourceProvider {...fixture}><Host /></DataSourceProvider>))
    await vi.waitFor(() => expect(container.querySelectorAll('[data-tool-status]')).toHaveLength(7))
    expect(container.querySelector('[data-item-id="inspector/raw"][data-tool-status]')).toBeNull()
    expect(within(container).getByRole('button', { name: /Show system events/ })).not.toBeNull()
    for (const kind of ['run', 'read', 'long-read', 'change', 'write', 'raw', 'running', 'failed']) {
      // A completed source update restores the ordinary folded work summary.
      await act(async () => container.querySelector<HTMLButtonElement>('[data-work-log-summary][aria-expanded="false"]')?.click())
      if (kind === 'raw') {
        await act(async () => within(container).getByRole('button', { name: /Show system events/ }).click())
        expect(container.querySelectorAll('[data-tool-status]')).toHaveLength(8)
      }
      await vi.waitFor(() => expect(container.querySelector(`[data-item-id="inspector/${kind}"][data-tool-status]`)).not.toBeNull())
      const row = container.querySelector(`[data-item-id="inspector/${kind}"][data-tool-status]`)!
      await act(async () => row.querySelector<HTMLButtonElement>(':scope > button')!.click())
      await act(async () => within(row as HTMLElement).getByRole('button', { name: /Open .* tool detail/ }).click())
      const panel = within(container).getByRole('complementary', { name: 'Tool detail' })
      expect(panel.querySelector('[data-tool-input]')).not.toBeNull()
      expect(within(panel).getByRole('region', { name: 'Tool output' })).not.toBeNull()
      expect(panel.querySelector('[data-tool-call-id]')?.textContent).toBe(kind)
      expect(panel.querySelector('[data-native-entry-id]')?.textContent).toBe(`inspector/${kind}`)
      expect(panel.querySelector('[data-tool-duration]')?.textContent).toBe(kind === 'running' ? 'Not reported' : '1.3 s')
      expect(panel.dataset.resultView).toBe(kind === 'failed' || kind === 'running' ? 'run' : kind === 'write' ? 'change' : kind === 'long-read' ? 'read' : kind)
      if (kind === 'run') expect(panel.querySelector('[data-testid=tool-inspector-command]')?.textContent).toBe('pnpm test rows')
      if (kind === 'read') expect(panel.querySelector('[data-tool-output]')?.textContent).toContain('export const rowKey')
      if (kind === 'long-read') {
        const result = within(panel).getByRole('region', { name: 'Tool output' })
        expect(result.textContent).not.toContain('export const row40')
        await act(async () => within(result).getByRole('button', { name: 'Show all 40 lines' }).click())
        expect(result.textContent).toContain('export const row40')
        await act(async () => within(result).getByRole('button', { name: 'Show 12-line preview' }).click())
        expect(result.textContent).not.toContain('export const row40')
      }
      if (kind === 'change') expect(within(panel).getByRole('region', { name: 'Changes to src/rows.ts' }).textContent).toContain('return row.id')
      if (kind === 'write') {
        expect(panel.querySelector('[data-recorded-change]')?.textContent).toBe('export const key = "row-a"')
        expect(panel.querySelector('[data-tool-output]')?.textContent).toBe('Wrote src/keys.ts.')
        expect(panel.querySelector('table')).toBeNull()
        expect(panel.querySelectorAll('[data-recorded-change]')).toHaveLength(1)
      }
      if (kind === 'raw') expect(panel.querySelector('[data-tool-output]')?.textContent).toContain('"revision": 4')
      if (kind === 'running') {
        expect(panel.querySelector('[data-tool-inspector-status]')?.textContent).toBe('Running')
        const items = inspectorItems.map(item => item.id !== 'inspector/running' ? item : { ...item, status: 'success' as const, result: { content: 'The delayed checks passed.', isError: false, at: '2026-01-15T12:00:03.000Z' } })
        await act(async () => fixture.setPage({ items, hasOlder: false }))
        expect(panel.querySelector('[data-tool-inspector-status]')?.textContent).toBe('Succeeded')
        expect(panel.querySelector('[data-tool-duration]')?.textContent).toBe('3.0 s')
        expect(panel.querySelector('[data-tool-output]')?.textContent).toBe('The delayed checks passed.')
      }
      expect(panel.querySelector('details')?.open).toBe(false)
      await act(async () => within(panel).getByRole('button', { name: 'Close tool detail' }).click())
      expect(within(container).queryByRole('complementary', { name: 'Tool detail' })).toBeNull()
      await act(async () => row.querySelector<HTMLButtonElement>(':scope > button')!.click())
      if (kind === 'raw') {
        await act(async () => within(container).getByRole('button', { name: /Hide system events/ }).click())
        expect(container.querySelector('[data-item-id="inspector/raw"][data-tool-status]')).toBeNull()
        expect(container.querySelectorAll('[data-tool-status]')).toHaveLength(7)
      }
    }
  } finally { await act(async () => root.unmount()); fixture.registry.dispose(); container.remove(); vi.unstubAllGlobals() }
})

it('never invents duration, input, output or an applied change for an incomplete failed call', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  const root = createRoot(container)
  try {
    await act(async () => root.render(<ToolInspector call={{ id: 'missing', kind: 'change', title: 'Changing rows', toolName: 'edit', startedAt: '', status: 'interrupted' }} onClose={() => {}} />))
    expect(container.querySelector('[data-tool-duration]')?.textContent).toBe('Not reported')
    expect(container.textContent).toContain('No invocation input available in loaded history.')
    expect(container.textContent).toContain('No output recorded.')
    expect(container.querySelector('[data-result-view]')?.getAttribute('data-result-view')).toBe('raw')
    expect(container.querySelector('table')).toBeNull()
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals() }
})

it('keeps the raw recorded output byte-exact even when it parses as round-trippable JSON', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  const root = createRoot(container)
  const output = '{"id":7,"rows":["a","b"]}'
  try {
    await act(async () => root.render(<ToolInspector call={{ id: 'raw-call', kind: 'unknown', title: 'Fictional fetch', toolName: 'fetch_rows', startedAt: '', status: 'success', input: { id: 1 }, output }} onClose={() => {}} />))
    expect(container.querySelector('[data-tool-raw] [data-tool-output]')?.textContent).toBe(output)
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals() }
})

const KeyboardInspectorHost = ({ manageFocus }: { readonly manageFocus: boolean }) => {
  const [open, setOpen] = React.useState(false)
  const Inspector = manageFocus ? ToolInspector : ToolInspectorContent
  return <><button onClick={() => setOpen(true)}>Inspect recorded call</button>{open && <Inspector call={{ id: 'keyboard-call', kind: 'run', title: 'Check fictional rows', toolName: 'run', startedAt: '', status: 'success', input: { command: 'printf rows' }, output: 'rows' }} onClose={() => setOpen(false)} />}</>
}

it.each([true, false])('Escape dismissal owns focus only with the managed inspector (manageFocus=%s)', async manageFocus => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 0))
  vi.stubGlobal('cancelAnimationFrame', (handle: number) => window.clearTimeout(handle))
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => root.render(<KeyboardInspectorHost manageFocus={manageFocus} />))
    const canvas = within(container)
    const opener = canvas.getByRole('button', { name: 'Inspect recorded call' })
    opener.focus()
    await act(async () => opener.click())
    const panel = canvas.getByRole('complementary', { name: 'Tool detail' })
    expect(panel.parentElement).toBe(container)
    const close = within(panel).getByRole('button', { name: 'Close tool detail' })
    if (manageFocus) await vi.waitFor(() => expect(panel.contains(document.activeElement)).toBe(true))
    close.focus()
    const key = async (value: string) => {
      const event = new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true })
      await act(async () => close.dispatchEvent(event))
      return event
    }
    expect((await key('ArrowDown')).defaultPrevented).toBe(false)
    expect(canvas.getByRole('complementary', { name: 'Tool detail' })).toBe(panel)
    const childEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') event.preventDefault() }
    close.addEventListener('keydown', childEscape)
    try {
      await key('Escape')
      expect(canvas.getByRole('complementary', { name: 'Tool detail' })).toBe(panel)
      expect(document.activeElement).toBe(close)
    } finally { close.removeEventListener('keydown', childEscape) }
    const escape = await key('Escape')
    if (manageFocus) {
      expect(escape.defaultPrevented).toBe(true)
      expect(canvas.queryByRole('complementary', { name: 'Tool detail' })).toBeNull()
      await vi.waitFor(() => expect(document.activeElement).toBe(opener))
    } else {
      // Negative control: content alone cannot satisfy the dismissal/focus contract.
      expect(escape.defaultPrevented).toBe(false)
      expect(canvas.getByRole('complementary', { name: 'Tool detail' })).toBe(panel)
      expect(document.activeElement).toBe(close)
      expect(document.activeElement).not.toBe(opener)
    }
  } finally { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals() }
})
