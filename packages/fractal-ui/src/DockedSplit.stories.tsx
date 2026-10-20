import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { DockedSplit, dockedSplitStorageKey } from './assistant-ui/composition/DockedSplit'
import { Pane, PaneHeaderToggle } from './assistant-ui/composition/Pane'
import { DiffPanel } from './assistant-ui/composition/DiffPanel'
import { Button } from './assistant-ui/composition/Controls'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { borderVars as border, geometryVars as g, geometryNumbers, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const workspaces = ['alpha', 'beta'] as const
type Workspace = typeof workspaces[number]
const persistKey = (workspace: Workspace) => `docked-story.${workspace}.changes`
const diff = ['@@ -12,3 +12,4 @@', '-const track = measure(rows)', '+const track = Math.max(minTrack, measure(rows))', '+cohort.style.setProperty(TRACK, `${track}px`)', ' return track']
/** Main-pane renders; a drag must not add any. */
const renders = { main: 0 }

function MainPane({ open, onOpenChange }: { readonly open: boolean; readonly onOpenChange: (open: boolean) => void }) {
  renders.main += 1
  return <Pane tabs={[{ key: 'thread', title: 'Fix flaky interaction gate' }]} activeKey="thread" onSelectTab={() => {}}
    actions={<PaneHeaderToggle label="Show changes" icon="panel" isSelected={open} onChange={onOpenChange} />}>
    <p data-main-body {...stylex.props(styles.body)}>Thread content keeps its DOM node while Changes opens and closes.</p>
  </Pane>
}

function DockedSplitStory({ scheme }: { readonly scheme: Scheme }) {
  const [workspace, setWorkspace] = React.useState<Workspace>('alpha')
  const [open, setOpen] = React.useState(true)
  const [generation, setGeneration] = React.useState(0)
  return <div data-scheme={scheme} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.root)}>
    <div role="toolbar" aria-label="Story controls" {...stylex.props(styles.toolbar)}>
      {workspaces.map(key => <Button key={key} aria-pressed={workspace === key} onPress={() => setWorkspace(key)}>{`Workspace ${key}`}</Button>)}
      <Button onPress={() => setGeneration(value => value + 1)}>Remount</Button>
    </div>
    <DockedSplit key={generation} persistKey={persistKey(workspace)} label="Resize changes" onCollapse={() => setOpen(false)}
      dock={open ? <DiffPanel open title="Captured changes" width="100%" diff={diff} path="src/sidebar/row-fit.ts" added={2} removed={1} onClose={() => setOpen(false)}
        info={<p {...stylex.props(styles.info)}>Successful-tool captures · loaded transcript window</p>} /> : undefined}>
      <MainPane open={open} onOpenChange={setOpen} />
    </DockedSplit>
  </div>
}

function BoundedSplitStory({ scheme }: { readonly scheme: Scheme }) {
  const [generation, setGeneration] = React.useState(0)
  const container = React.useRef<HTMLDivElement>(null)
  const sidebar = React.useRef<HTMLElement>(null)
  return <div data-scheme={scheme} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.root)}>
    <div role="toolbar" aria-label="Story controls" {...stylex.props(styles.toolbar)}>
      <Button onPress={() => { container.current!.style.width = '900px' }}>900px window</Button>
      <Button onPress={() => { container.current!.style.width = '1440px' }}>1440px window</Button>
      <Button onPress={() => { sidebar.current!.style.width = '256px' }}>256px sidebar</Button>
      <Button onPress={() => { window.localStorage.setItem(dockedSplitStorageKey('bounded-story'), '720'); setGeneration(value => value + 1) }}>Restore 720px</Button>
    </div>
    <div ref={container} style={{ display: 'flex', width: 1440, flex: 1, minHeight: 0 }}>
      <aside ref={sidebar} style={{ width: 208, flexShrink: 0 }}>Sidebar</aside>
      <DockedSplit key={generation} persistKey="bounded-story" label="Resize changes" minMainSize={geometryNumbers.panelMin} dock={<DiffPanel open width="100%" diff={diff} path="src/sidebar/row-fit.ts" added={2} removed={1} />}>
        <p data-bounded-thread>Thread and composer stay usable.</p>
      </DockedSplit>
    </div>
  </div>
}

const meta = {
  title: 'Fractal UI/Docked split',
  component: DockedSplitStory,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Main pane with a right-docked, resizable Changes panel: 320–720 px, persisted per workspace key, sized through a CSS variable so a drag never re-renders the host. Pane headers share the 32px geometry.' } } },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  beforeEach: () => { for (const workspace of workspaces) window.localStorage.removeItem(dockedSplitStorageKey(persistKey(workspace))); window.localStorage.removeItem(dockedSplitStorageKey('bounded-story')) },
} satisfies Meta<typeof DockedSplitStory>
export default meta
type Story = StoryObj<typeof meta>

const separator = (canvasElement: HTMLElement) => within(canvasElement).getByRole('separator', { name: 'Resize changes' })
const dockWidth = (canvasElement: HTMLElement) => canvasElement.querySelector<HTMLElement>('[data-dock]')!.getBoundingClientRect().width
const stored = (workspace: Workspace) => window.localStorage.getItem(dockedSplitStorageKey(persistKey(workspace)))
const drag = (node: HTMLElement, fromX: number, toX: number, finish: 'pointerup' | 'pointercancel' = 'pointerup') => {
  const init = { bubbles: true, cancelable: true, composed: true, pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 }
  node.dispatchEvent(new PointerEvent('pointerdown', { ...init, buttons: 1, clientX: fromX }))
  node.dispatchEvent(new PointerEvent('pointermove', { ...init, buttons: 1, clientX: (fromX + toX) / 2 }))
  node.dispatchEvent(new PointerEvent('pointermove', { ...init, buttons: 1, clientX: toX }))
  return () => node.dispatchEvent(new PointerEvent(finish, { ...init, buttons: 0, clientX: toX }))
}

export const Default: Story = {}
export const DefaultLight: Story = { args: { scheme: 'light' } }

export const AvailableWidth: Story = {
  render: args => <BoundedSplitStory {...args} />,
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const thread = canvasElement.querySelector('[data-bounded-thread]')!.parentElement!
    const handle = () => separator(canvasElement)
    handle().focus()
    await userEvent.keyboard('{End}')
    await expect(handle()).toHaveAttribute('aria-valuenow', '720')
    await userEvent.click(canvas.getByRole('button', { name: '900px window' }))
    await waitFor(() => expect(handle()).toHaveAttribute('aria-valuemax', '331'))
    await expect(thread.getBoundingClientRect().width).toBeGreaterThanOrEqual(360)
    await expect(window.localStorage.getItem(dockedSplitStorageKey('bounded-story'))).toBe('720')
    handle().focus()
    await userEvent.keyboard('{End}')
    await expect(handle()).toHaveAttribute('aria-valuenow', '331')
    await userEvent.click(canvas.getByRole('button', { name: 'Restore 720px' }))
    await expect(handle()).toHaveAttribute('aria-valuenow', '331')
    await expect(window.localStorage.getItem(dockedSplitStorageKey('bounded-story'))).toBe('720')
    await userEvent.click(canvas.getByRole('button', { name: '256px sidebar' }))
    await waitFor(() => expect(handle()).toHaveAttribute('aria-valuemax', '283'))
    await expect(canvasElement.querySelector('[data-bounded-thread]')!.parentElement!.getBoundingClientRect().width).toBeGreaterThanOrEqual(360)
    await expect(handle()).toHaveAttribute('aria-valuemin', '283')
    await expect(window.localStorage.getItem(dockedSplitStorageKey('bounded-story'))).toBe('720')
    await userEvent.click(canvas.getByRole('button', { name: '1440px window' }))
    await waitFor(() => expect(handle()).toHaveAttribute('aria-valuenow', '720'))
    await expect(handle()).toHaveAttribute('aria-valuemax', '720')
  },
}

/** Dragging previews the width with no host re-render, commits once on pointerup, clamps at both bounds and cancels cleanly. */
export const PointerResize: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const handle = separator(canvasElement)
    await expect(handle).toHaveAttribute('aria-valuenow', '440')
    await expect(handle).toHaveAttribute('aria-valuemin', '320')
    await expect(handle).toHaveAttribute('aria-valuemax', '720')
    await expect(handle).toHaveAttribute('aria-orientation', 'vertical')
    await expect(dockWidth(canvasElement)).toBe(440)
    const x = handle.getBoundingClientRect().left
    const before = renders.main
    // Leftward widens a right dock.
    const release = drag(handle, x, x - 100)
    await waitFor(() => expect(handle).toHaveAttribute('aria-valuenow', '540'))
    await expect(dockWidth(canvasElement)).toBe(540)
    await expect(renders.main).toBe(before)
    await expect(stored('alpha')).toBeNull()
    release()
    await expect(stored('alpha')).toBe('540')
    await expect(renders.main).toBe(before)
    // Past either bound the width clamps.
    drag(handle, x, x - 2000)()
    await waitFor(() => expect(handle).toHaveAttribute('aria-valuenow', '720'))
    await expect(dockWidth(canvasElement)).toBe(720)
    await expect(stored('alpha')).toBe('720')
    drag(handle, x, x + 2000)()
    await waitFor(() => expect(handle).toHaveAttribute('aria-valuenow', '320'))
    await expect(dockWidth(canvasElement)).toBe(320)
    await expect(stored('alpha')).toBe('320')
    // A cancelled drag restores the committed width and stores nothing new.
    const cancel = drag(handle, x, x - 200, 'pointercancel')
    await waitFor(() => expect(handle).toHaveAttribute('aria-valuenow', '520'))
    cancel()
    await expect(handle).toHaveAttribute('aria-valuenow', '320')
    await expect(dockWidth(canvasElement)).toBe(320)
    await expect(stored('alpha')).toBe('320')
    await expect(renders.main).toBe(before)
  },
}

/** Arrows step ±8 (Shift ±32) with ← widening, Home/End jump to the bounds, steps clamp there, and Enter collapses. */
export const KeyboardResize: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const handle = separator(canvasElement)
    handle.focus()
    await userEvent.keyboard('{ArrowLeft}')
    await expect(handle).toHaveAttribute('aria-valuenow', '448')
    await userEvent.keyboard('{Shift>}{ArrowLeft}{/Shift}')
    await expect(handle).toHaveAttribute('aria-valuenow', '480')
    await userEvent.keyboard('{ArrowRight}')
    await expect(handle).toHaveAttribute('aria-valuenow', '472')
    await expect(dockWidth(canvasElement)).toBe(472)
    await expect(stored('alpha')).toBe('472')
    await userEvent.keyboard('{Home}')
    await expect(handle).toHaveAttribute('aria-valuenow', '320')
    await userEvent.keyboard('{ArrowRight}')
    await expect(handle).toHaveAttribute('aria-valuenow', '320')
    await userEvent.keyboard('{End}')
    await expect(handle).toHaveAttribute('aria-valuenow', '720')
    await userEvent.keyboard('{Shift>}{ArrowLeft}{/Shift}')
    await expect(handle).toHaveAttribute('aria-valuenow', '720')
    await expect(dockWidth(canvasElement)).toBe(720)
    await expect(stored('alpha')).toBe('720')
    const body = canvasElement.querySelector('[data-main-body]')
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(within(canvasElement).queryByRole('separator', { name: 'Resize changes' })).toBeNull())
    await expect(canvasElement.querySelector('[data-dock]')).toBeNull()
    // The main pane keeps its node while the dock goes away.
    await expect(canvasElement.querySelector('[data-main-body]')).toBe(body)
  },
}

/** A committed width survives closing the dock and remounting the split; a corrupt or out-of-range entry clamps. */
export const Persistence: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const body = canvasElement.querySelector('[data-main-body]')
    separator(canvasElement).focus()
    await userEvent.keyboard('{Shift>}{ArrowLeft}{ArrowLeft}{/Shift}')
    await expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '504')
    await userEvent.click(canvas.getByRole('button', { name: 'Show changes' }))
    await expect(canvas.queryByRole('separator', { name: 'Resize changes' })).toBeNull()
    await expect(canvasElement.querySelector('[data-main-body]')).toBe(body)
    await userEvent.click(canvas.getByRole('button', { name: 'Show changes' }))
    await expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '504')
    await expect(dockWidth(canvasElement)).toBe(504)
    await expect(canvasElement.querySelector('[data-main-body]')).toBe(body)
    await userEvent.click(canvas.getByRole('button', { name: 'Remount' }))
    await waitFor(() => expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '504'))
    await expect(dockWidth(canvasElement)).toBe(504)
    window.localStorage.setItem(dockedSplitStorageKey(persistKey('alpha')), '9000')
    await userEvent.click(canvas.getByRole('button', { name: 'Remount' }))
    await waitFor(() => expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '720'))
    window.localStorage.setItem(dockedSplitStorageKey(persistKey('alpha')), 'wide')
    await userEvent.click(canvas.getByRole('button', { name: 'Remount' }))
    await waitFor(() => expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '440'))
  },
}

/** Each workspace key keeps its own width; switching restores that workspace's width without touching the other. */
export const WorkspaceIsolation: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    separator(canvasElement).focus()
    await userEvent.keyboard('{End}')
    await expect(stored('alpha')).toBe('720')
    await userEvent.click(canvas.getByRole('button', { name: 'Workspace beta' }))
    await waitFor(() => expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '440'))
    await expect(dockWidth(canvasElement)).toBe(440)
    separator(canvasElement).focus()
    await userEvent.keyboard('{Home}')
    await expect(stored('beta')).toBe('320')
    await expect(stored('alpha')).toBe('720')
    await userEvent.click(canvas.getByRole('button', { name: 'Workspace alpha' }))
    await waitFor(() => expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '720'))
    await expect(dockWidth(canvasElement)).toBe(720)
    await userEvent.click(canvas.getByRole('button', { name: 'Workspace beta' }))
    await waitFor(() => expect(separator(canvasElement)).toHaveAttribute('aria-valuenow', '320'))
    await expect(dockWidth(canvasElement)).toBe(320)
  },
}

/** Thread and Changes headers align; source information is disclosed from the toolbar. */
export const SharedHeaders: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    const main = canvasElement.querySelector<HTMLElement>('[data-testid="pane-header"]')!
    const dock = canvasElement.querySelector<HTMLElement>('[data-testid="diff-panel-header"]')!
    await expect(main.getBoundingClientRect().height).toBe(32)
    await expect(dock.getBoundingClientRect().height).toBe(32)
    await expect(dock.getBoundingClientRect().top).toBe(main.getBoundingClientRect().top)
    const info = canvasElement.querySelector<HTMLElement>('[data-testid="diff-panel-info"]')!
    await expect(info).not.toBeVisible()
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Captured changes information' }))
    await expect(info).toBeVisible()
    await expect(info.getBoundingClientRect().top).toBeGreaterThanOrEqual(dock.getBoundingClientRect().bottom)
    await expect(within(canvasElement).getByRole('button', { name: 'Close Captured changes' })).toBeVisible()
  },
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', height: '100vh', minWidth: 0, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  toolbar: { display: 'flex', alignItems: 'center', gap: s.xs, paddingInline: s.sm, paddingBlock: s.xs, flexShrink: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  body: { margin: 0, padding: s.lg, color: ink.fgSoft },
  info: { margin: 0, paddingInline: s.lg, paddingBlock: s.sm, color: ink.fgMuted, fontSize: t.denseSize },
})
