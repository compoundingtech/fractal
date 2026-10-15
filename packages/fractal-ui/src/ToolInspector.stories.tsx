import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import type { WorkLogCall } from './assistant-ui/taste/work-log.ts'
import { ConversationPane } from '../../../apps/fractal-web/src/web/ConversationPane.tsx'
import { LiveToolInspector, ToolInspectorContent } from '../../../apps/fractal-web/src/web/ToolInspector.tsx'
import { DataSourceProvider } from '../../../apps/fractal-web/src/data/react.tsx'
import { createToolInspectorFixture, inspectorFixtureRef } from '../../../apps/fractal-web/src/data/toolInspectorFixture.ts'
import { liveLegacyTheme } from '../../../apps/fractal-web/src/ui-compat/live-theme.stylex.ts'
import { assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme, compositionLightTheme } from './assistant-ui/shell.ts'
import { colorVars as c } from './assistant-ui/composition-tokens.stylex.ts'

const InspectorSurface = ({ scheme = 'dark', width = 1440, inspector = true, manageFocus = true }: { readonly scheme?: 'dark' | 'light'; readonly width?: number; readonly inspector?: boolean; readonly manageFocus?: boolean }) => {
  const fixture = React.useMemo(createToolInspectorFixture, [])
  const [opened, setOpened] = React.useState<WorkLogCall>()
  return <div data-tool-inspector-story="" data-scheme={scheme} {...stylex.props(liveLegacyTheme, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)} style={{ backgroundColor: c.canvas, color: c.fg, width: Math.min(width, window.innerWidth), height: 900, display: 'flex' }}>
    <DataSourceProvider source={fixture.source} registry={fixture.registry}>
      <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minWidth: 0 }}><ConversationPane agentRef={inspectorFixtureRef} agentName="Row review" onOpenTool={setOpened} /></div>
      {inspector && opened !== undefined && (manageFocus ? <LiveToolInspector agentRef={inspectorFixtureRef} call={opened} width={380} onClose={() => setOpened(undefined)} /> : <ToolInspectorContent call={opened} width={380} onClose={() => setOpened(undefined)} />)}
    </DataSourceProvider>
  </div>
}
const inspectorPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const root = canvasElement.querySelector<HTMLElement>('[data-tool-inspector-story]')!
  const canvas = within(root)
  await canvas.findByText('Checking row selection')
  const rows = root.querySelectorAll<HTMLButtonElement>('[data-work-log-summary][aria-expanded="false"]')
  for (const row of rows) await userEvent.click(row)
  for (const kind of ['run', 'read', 'long-read', 'change', 'write', 'raw', 'running', 'failed']) {
    const row = root.querySelector<HTMLElement>(`[data-item-id="inspector/${kind}"][data-tool-status]`)!
    await userEvent.click(row.querySelector<HTMLButtonElement>(':scope > button')!)
    if (kind === 'run') await expect(within(row).getByTestId('tool-command')).toHaveTextContent('pnpm test rows')
    const opener = within(row).getByRole('button', { name: /Open .* tool detail/ })
    await userEvent.click(opener)
    const panel = await canvas.findByRole('complementary', { name: 'Tool detail' })
    const detail = within(panel)
    await waitFor(() => expect(panel.contains(document.activeElement)).toBe(true))
    await expect(panel.querySelector('[data-tool-call-id]')).toHaveTextContent(kind)
    await expect(panel.querySelector('[data-native-entry-id]')).toHaveTextContent(`inspector/${kind}`)
    await expect(detail.getByRole('region', { name: 'Tool input' })).toBeInTheDocument()
    await expect(detail.getByRole('region', { name: 'Tool output' })).toBeInTheDocument()
    await expect(panel.querySelector('[data-tool-duration]')).toHaveTextContent(kind === 'running' ? 'Not reported' : '1.3 s')
    await expect(panel.querySelector('[data-tool-name]')).toHaveTextContent(kind === 'change' ? 'edit' : kind === 'write' ? 'write' : kind === 'raw' ? 'catalog.inspect' : kind === 'read' || kind === 'long-read' ? 'read' : 'run')
    if (kind === 'run') await expect(detail.getByTestId('tool-inspector-command')).toHaveTextContent('pnpm test rows')
    if (kind === 'read') await expect(panel.querySelector('[data-tool-output]')).toHaveTextContent('export const rowKey')
    if (kind === 'long-read') {
      const result = detail.getByRole('region', { name: 'Tool output' })
      const height = result.getBoundingClientRect().height
      await expect(result.textContent).not.toContain('export const row40')
      await userEvent.click(within(result).getByRole('button', { name: 'Show all 40 lines' }))
      await expect(result).toHaveTextContent('export const row40')
      await expect(result.getBoundingClientRect().height).toBe(height)
      await userEvent.click(within(result).getByRole('button', { name: 'Show 12-line preview' }))
      await expect(result.getBoundingClientRect().height).toBe(height)
    }
    if (kind === 'change') await expect(detail.getByRole('region', { name: 'Changes to src/rows.ts' })).toBeInTheDocument()
    if (kind === 'write') {
      await expect(detail.getByText('Recorded write/edit source')).toBeInTheDocument()
      await expect(panel.querySelector('[data-recorded-change]')).toHaveTextContent('export const key = "row-a"')
      await expect(panel.querySelector('[data-tool-output]')).toHaveTextContent('Wrote src/keys.ts.')
      await expect(panel.querySelector('table')).toBeNull()
    }
    if (kind === 'raw') await expect(detail.getByRole('heading', { name: 'Raw output · no specialized renderer' })).toBeInTheDocument()
    if (kind === 'running') await expect(detail.getByText('No output received yet.')).toBeInTheDocument()
    if (kind === 'failed') await expect(panel.querySelector('[data-tool-inspector-status]')).toHaveTextContent('Failed')
    await userEvent.click(detail.getByText('Raw recorded output'))
    await expect(panel.querySelector('details')).toHaveAttribute('open')
    await userEvent.click(detail.getByRole('button', { name: 'Close tool detail' }))
    await expect(canvas.queryByRole('complementary', { name: 'Tool detail' })).toBeNull()
    await waitFor(() => expect(opener).toHaveFocus())
    await userEvent.click(row.querySelector<HTMLButtonElement>(':scope > button')!)
  }
  const runRow = root.querySelector<HTMLElement>('[data-item-id="inspector/run"][data-tool-status]')!
  await userEvent.click(runRow.querySelector<HTMLButtonElement>(':scope > button')!)
  await userEvent.click(within(runRow).getByRole('button', { name: /Open .* tool detail/ }))
  await canvas.findByRole('complementary', { name: 'Tool detail' })
  root.dataset.proof = 'passed'
}
const escapePlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const root = canvasElement.querySelector<HTMLElement>('[data-tool-inspector-story]')!
  const canvas = within(root)
  await canvas.findByText('Checking row selection')
  for (const summary of root.querySelectorAll<HTMLButtonElement>('[data-work-log-summary][aria-expanded="false"]')) await userEvent.click(summary)
  const row = root.querySelector<HTMLElement>('[data-item-id="inspector/run"][data-tool-status]')!
  await userEvent.click(row.querySelector<HTMLButtonElement>(':scope > button')!)
  const opener = within(row).getByRole('button', { name: /Open .* tool detail/ })
  await userEvent.click(opener)
  const panel = await canvas.findByRole('complementary', { name: 'Tool detail' })
  await expect(panel.parentElement).toBe(root)
  // Explicit focus also lets the content-only control reach the keyboard assertions.
  const close = within(panel).getByRole('button', { name: 'Close tool detail' })
  close.focus()
  await userEvent.keyboard('{ArrowDown}')
  await expect(panel).toBeInTheDocument()
  const handleChildEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') event.preventDefault() }
  close.addEventListener('keydown', handleChildEscape)
  try {
    await userEvent.keyboard('{Escape}')
    await expect(panel).toBeInTheDocument()
    await expect(close).toHaveFocus()
  } finally { close.removeEventListener('keydown', handleChildEscape) }
  await userEvent.keyboard('{Escape}')
  await expect(canvas.queryByRole('complementary', { name: 'Tool detail' })).toBeNull()
  await waitFor(() => expect(opener).toHaveFocus())
}

const meta = { title: 'Fractal/App/Transcript/Tool inspector', component: InspectorSurface, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof InspectorSurface>
export default meta
type Story = StoryObj<typeof meta>
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: inspectorPlay }
export const UnmanagedFocusControl: Story = { args: { manageFocus: false }, play: inspectorPlay }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: inspectorPlay }
export const Dark900: Story = { args: { scheme: 'dark', width: 900 }, play: inspectorPlay }
export const Light900: Story = { args: { scheme: 'light', width: 900 }, play: inspectorPlay }
export const MissingInspectorControl: Story = { args: { inspector: false }, play: inspectorPlay }
export const EscapeDismissal: Story = { play: escapePlay }
export const EscapeUnmanagedFocusControl: Story = { args: { manageFocus: false }, play: escapePlay }
export const AllStates: Story = { render: () => <><InspectorSurface width={900} /><InspectorSurface width={900} scheme="light" /></> }
