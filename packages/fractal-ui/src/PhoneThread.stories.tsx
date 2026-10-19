import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { ThreadHeader } from './assistant-ui/composition/Shell'
import { NarrowLayoutContext } from './assistant-ui/composition/NarrowLayout'
import { Transcript, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { darkTheme, liveAccentTheme, liveComposerDarkTheme } from './assistant-ui/embrace-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const now = Date.parse('2026-01-15T12:04:12Z')
const startedAt = '2026-01-15T12:00:00Z'
const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: 'phone/prompt', role: 'user', text: 'Check the row projection on a phone-sized window.', attachments: [], streaming: false, at: startedAt, sender: { kind: 'human', label: 'Operator' } }
const reply: TextItem = { _tag: 'Text', id: 'phone/reply', role: 'assistant', text: 'Looking at the narrow layout now.', attachments: [], streaming: false, at: '2026-01-15T12:00:10Z', sender: { kind: 'agent', label: 'Assistant' } }
const turn: TranscriptTurn = { id: 'phone', prompt, items: [reply], work: workLogTurnFromItems([prompt, reply], { kindFor: () => 'read', running: true, failed: false, interrupted: false, startedAt, completeHistory: true }) }
const sync = { _tag: 'Live', since: now - 5000 } as const

/**
 * The thread as a phone host composes it: header, transcript and the C1 composer in one flex column sized to the
 * dynamic viewport, with `NarrowLayoutContext` set. The composer stays in flow under the transcript (never fixed),
 * so when the keyboard shrinks the dynamic viewport the transcript gives up height and Send stays on screen.
 * With `narrow` off the same column keeps desktop density.
 */
function PhoneThreadStory({ scheme, narrow }: { scheme: Scheme; narrow: boolean }) {
  const options = React.useMemo(() => ({ messages: [prompt, reply], isRunning: true, onNew: async () => {}, onCancel: async () => {} }), [])
  return <main aria-label="Thread" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'dark' && darkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && lightTheme)}>
    <NarrowLayoutContext.Provider value={narrow}>
      <div data-testid="phone-column" {...stylex.props(styles.column)}>
        <EmbraceRuntimeProvider options={options}>
          <ThreadHeader narrow={narrow} folder="build-1" title="Row projection" status="working" statusLabel="working" freshness="live" sidebarCollapsed={narrow} onToggleSidebar={() => {}} panelOpen={false} drawerOpen={false} onTogglePanel={() => {}} onToggleDrawer={() => {}} />
          <div {...stylex.props(styles.transcript)}><Transcript viewportKey="phone" title="Row projection" turns={[turn]} sync={sync} now={now} observedAt={now - 3000} onOpenTool={() => {}} /></div>
          <div data-testid="story-composer-dock" {...stylex.props(styles.dock)}><EmbraceComposer variant="C1" readingColumn conversationLabel="Row projection" /></div>
        </EmbraceRuntimeProvider>
      </div>
    </NarrowLayoutContext.Provider>
  </main>
}
const meta = {
  title: 'Fractal UI/Phone thread',
  component: PhoneThreadStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', narrow: true },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, narrow: { control: 'boolean' } },
} satisfies Meta<typeof PhoneThreadStory>
export default meta
type Story = StoryObj<typeof meta>

/** No ancestor of the composer takes it out of flow: a fixed or sticky composer would sit under the keyboard. */
const inFlow = (element: HTMLElement, root: HTMLElement) => {
  for (let node: HTMLElement | null = element; node !== null && node !== root; node = node.parentElement) {
    const position = getComputedStyle(node).position
    if (position === 'fixed' || position === 'sticky') return false
  }
  return true
}

export const Narrow: Story = { tags: ['critical'], play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const column = canvas.getByTestId('phone-column')
  const composer = canvas.getByRole('form', { name: 'Message composer, Row projection' })
  await expect(canvasElement.querySelector('[data-testid="kit-composer"]')).toHaveAttribute('data-narrow', 'true')
  await expect(inFlow(composer, column)).toBe(true)
  const field = canvas.getByRole('textbox', { name: 'Message' })
  // 16px field text: iOS Safari zooms the page when a smaller field takes focus.
  await expect(getComputedStyle(field).fontSize).toBe('16px')
  const header = canvas.getByTestId('thread-header')
  for (const name of ['Expand sidebar', 'Agent details']) {
    const rect = within(header).getByRole('button', { name }).getBoundingClientRect()
    await expect(Math.min(rect.width, rect.height)).toBeGreaterThanOrEqual(44)
  }
  const stop = await waitFor(() => { const button = canvasElement.querySelector<HTMLButtonElement>('[data-testid="run-stop"]'); if (button === null) throw new Error('Stop is not rendered'); return button })
  await expect(stop.getBoundingClientRect().height).toBeGreaterThanOrEqual(44)
  const send = canvas.getByRole('button', { name: 'Send' })
  await expect(send.getBoundingClientRect().width).toBeGreaterThanOrEqual(44)
  await expect(send.getBoundingClientRect().height).toBeGreaterThanOrEqual(44)
  // A long draft grows the field to at most a third of the dynamic viewport, then scrolls inside it; Send stays on screen.
  await userEvent.click(field)
  await userEvent.type(field, Array.from({ length: 24 }, (_, line) => `line ${line + 1}`).join('{Shift>}{Enter}{/Shift}'))
  await waitFor(() => expect(field.scrollHeight).toBeGreaterThan(field.clientHeight))
  await expect(field.getBoundingClientRect().height).toBeLessThanOrEqual(Math.min(208, window.innerHeight / 3) + 1)
  await expect(send.getBoundingClientRect().bottom).toBeLessThanOrEqual(column.getBoundingClientRect().bottom)
  await expect(send.getBoundingClientRect().bottom).toBeLessThanOrEqual(window.innerHeight)
  await expect(header.getBoundingClientRect().top).toBeGreaterThanOrEqual(column.getBoundingClientRect().top)
  // iOS can shrink only the visual viewport: use the host's explicit height, not layout dvh.
  column.style.setProperty('--wf-phone-height', '400px')
  column.style.height = '400px'
  await waitFor(() => expect(field.getBoundingClientRect().height).toBeLessThanOrEqual(400 / 3 + 1))
  await expect(send.getBoundingClientRect().bottom).toBeLessThanOrEqual(column.getBoundingClientRect().bottom)
} }
export const NarrowLight: Story = { ...Narrow, args: { scheme: 'light', narrow: true } }
/** Desktop density is unchanged outside a narrow host: 14px field text and the 32px Send. */
export const Desktop: Story = { tags: ['critical'], args: { narrow: false }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(canvasElement.querySelector('[data-testid="kit-composer"]')).not.toHaveAttribute('data-narrow')
  await expect(getComputedStyle(canvas.getByRole('textbox', { name: 'Message' })).fontSize).toBe('14px')
  const send = canvas.getByRole('button', { name: 'Send' }).getBoundingClientRect()
  await expect({ width: send.width, height: send.height }).toEqual({ width: 32, height: 32 })
  await expect(within(canvas.getByTestId('thread-header')).getByRole('button', { name: 'Toggle right panel' }).getBoundingClientRect().height).toBe(28)
} }

const styles = stylex.create({
  root: { height: '100dvh', width: '100%', boxSizing: 'border-box', display: 'flex', justifyContent: 'center', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  // A 390px phone column; the gate's 1440 case keeps the same column so both widths measure a phone.
  column: { width: '390px', maxWidth: '100%', height: '100%', display: 'flex', flexDirection: 'column', minHeight: 0 },
  transcript: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' },
  dock: { flexShrink: 0, paddingBottom: s.lg },
})
