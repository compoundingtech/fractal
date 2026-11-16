import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { EmbraceScrollViewport } from './assistant-ui/EmbraceScrollViewport'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { interactionVars as interaction, surfaceVars as surface, textVars as ink, borderVars as border, spaceVars as s, typeVars as t, geometryVars as g, motionNumbers } from './assistant-ui/composition-tokens.stylex'

const streamLines = 60

function ScrollStory({ scheme = 'dark' }: { scheme?: Scheme }) {
  const [count, setCount] = React.useState(40)
  const [narrow, setNarrow] = React.useState(false)
  // A long reply streaming into the last row, one line per frame, like a tool's output.
  const [streamed, setStreamed] = React.useState(0)
  const stream = () => {
    let lines = 0
    const step = () => { setStreamed(++lines); if (lines < streamLines) requestAnimationFrame(step) }
    requestAnimationFrame(step)
  }
  const rows = React.useMemo(() => Array.from({ length: count }, (_, index) => ({ id: `entry-${index}`, version: index === count - 1 ? String(streamed) : undefined, text: `Observed message ${index + 1}: preserve the reader's position as the conversation grows or the viewport changes width.` })), [count, streamed])
  return <main {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><div {...stylex.props(styles.actions)}><Button onPress={() => setCount(value => value + 1)} {...stylex.props(styles.button)}>Append message</Button><Button onPress={() => setNarrow(value => !value)} {...stylex.props(styles.button)}>Change width</Button><Button onPress={stream} {...stylex.props(styles.button)}>Stream reply</Button></div><section {...stylex.props(styles.frame, narrow && styles.narrow)}><EmbraceScrollViewport items={rows} data-testid="scroll-viewport" aria-label="Conversation history" tabIndex={0} {...stylex.props(styles.viewport)} contentProps={stylex.props(styles.rows)}>{rows.map((row, index) => <article key={row.id} data-item-id={row.id} {...stylex.props(styles.row)}>{row.text}{index === rows.length - 1 && streamed > 0 && <pre data-testid="streamed-block" {...stylex.props(styles.streamed)}>{Array.from({ length: streamed }, (_, line) => `streamed line ${line + 1}`).join('\n')}</pre>}</article>)}</EmbraceScrollViewport></section></main>
}
const meta = { excludeStories: ['scenarioExemptions'], title: 'Fractal UI/Scroll Viewport', component: ScrollStory, args: { scheme: 'dark' }, parameters: { scenario: { _tag: 'primitive', reason: "Controlled row appends and viewport geometry isolate follow and anchor behavior." }, layout: 'fullscreen' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof ScrollStory>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:primitive',
  'AllStatesLight:primitive',
  'EasedReturnYieldsToReader:primitive',
  'EasedReturnYieldsToReaderLight:primitive',
  'FollowingAndAnchored:primitive',
  'FollowingAndAnchoredLight:primitive',
  'FollowsTheLiveEnd:primitive',
  'FollowsTheLiveEndLight:primitive',
]
type Story = StoryObj<typeof meta>
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
const frameTime = () => new Promise<number>(resolve => requestAnimationFrame(resolve))
/** Resolves on the first frame at least `duration` of frame time after the next one. Frames requested earlier run first in each frame. */
async function framesSpanning(duration: number) {
  const first = await frameTime()
  for (let now = first; now - first < duration;) now = await frameTime()
}
export const FollowingAndAnchored: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const viewport = canvas.getByTestId('scroll-viewport')
  await frame()
  await expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1)
  await userEvent.click(canvas.getByRole('button', { name: 'Append message' }))
  await frame()
  await expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1)
  viewport.focus()
  viewport.scrollTop = 240
  viewport.dispatchEvent(new Event('scroll'))
  await frame()
  const anchor = Array.from(viewport.querySelectorAll<HTMLElement>('[data-item-id]')).find(row => row.getBoundingClientRect().bottom > viewport.getBoundingClientRect().top)!
  const offset = anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top
  await userEvent.click(canvas.getByRole('button', { name: 'Append message' }))
  await frame()
  await expect(Math.abs(anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top - offset)).toBeLessThanOrEqual(1)
  await expect(canvas.getByRole('button', { name: 'New messages ↓' })).toBeVisible()
  await userEvent.click(canvas.getByRole('button', { name: 'Change width' }))
  await frame()
  // ResizeObserver schedules compensation on the next frame; two frames alone race that work.
  await waitFor(() => expect(Math.abs(anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top - offset)).toBeLessThanOrEqual(1))
  await userEvent.click(canvas.getByRole('button', { name: 'New messages ↓' }))
  // The return eases over the standard motion duration (200ms) of frame time from its first frame, or lands at once under
  // reduced motion. The jump requested that frame before this play does, so the lane has landed once the play's frames span
  // the duration. A loaded host delays or pauses frames while timers run, so a wall-clock bound failed a lane on course (#4469).
  await framesSpanning(motionNumbers.standard)
  await expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1)
  await expect(canvas.queryByRole('button', { name: 'New messages ↓' })).toBeNull()
} }
export const FollowingAndAnchoredLight: Story = { ...FollowingAndAnchored, args: { scheme: 'light' } }
/** The return to the end eases and yields to the reader's wheel; under reduced motion it lands at once. */
export const EasedReturnYieldsToReader: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const viewport = canvas.getByTestId('scroll-viewport')
  const distance = () => viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop
  await frame()
  viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 }))
  viewport.scrollTop = 0
  viewport.dispatchEvent(new Event('scroll'))
  await frame()
  await userEvent.click(canvas.getByRole('button', { name: 'Append message' }))
  const jump = await canvas.findByRole('button', { name: 'New messages ↓' })
  await userEvent.click(jump)
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    await frame()
    await expect(distance(), 'reduced motion returns at once').toBeLessThanOrEqual(1)
    return
  }
  // One frame into the ease the lane has started moving but has not landed.
  await new Promise(resolve => requestAnimationFrame(resolve))
  await expect(distance(), 'return jumped instead of easing').toBeGreaterThan(1)
  viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -40 }))
  const interrupted = viewport.scrollTop
  await frame()
  await frame()
  await expect(Math.abs(viewport.scrollTop - interrupted), 'eased return kept moving after reader input').toBeLessThanOrEqual(1)
  await expect(distance()).toBeGreaterThan(1)
} }
export const EasedReturnYieldsToReaderLight: Story = { ...EasedReturnYieldsToReader, args: { scheme: 'light' } }
/**
 * At the end the lane follows: through input that cannot scroll (a downward wheel, as trackpad momentum sends, a click),
 * new rows, and a long block streaming into the last row, whose newest line stays in view. A deliberate scroll up stops
 * following; the jump control resumes it.
 */
export const FollowsTheLiveEnd: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const viewport = canvas.getByTestId('scroll-viewport')
  const distance = () => viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop
  const append = async () => { await userEvent.click(canvas.getByRole('button', { name: 'Append message' })); await frame() }
  await frame()
  await expect(distance()).toBeLessThanOrEqual(1)
  viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: 300, bubbles: true }))
  await append()
  await expect(distance(), 'a downward wheel at the end stopped following').toBeLessThanOrEqual(1)
  await userEvent.click(viewport.querySelectorAll('[data-item-id]')[viewport.querySelectorAll('[data-item-id]').length - 1]!)
  await append()
  await expect(distance(), 'a click in the lane stopped following').toBeLessThanOrEqual(1)
  await userEvent.click(canvas.getByRole('button', { name: 'Stream reply' }))
  let worst = 0
  for (let index = 0; index < streamLines + 4; index++) {
    await new Promise(resolve => requestAnimationFrame(resolve))
    worst = Math.max(worst, distance())
  }
  await waitFor(() => expect(canvas.getByTestId('streamed-block').textContent).toContain(`streamed line ${streamLines}`))
  await frame()
  await expect(distance(), 'the newest streamed line left the view').toBeLessThanOrEqual(1)
  // One line of slack at most while a frame's growth is pinned.
  await expect(worst, 'streaming fell more than a line behind the end').toBeLessThanOrEqual(48)
  const last = canvas.getByTestId('streamed-block').getBoundingClientRect()
  await expect(last.bottom).toBeLessThanOrEqual(viewport.getBoundingClientRect().bottom + 1)
  viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -400, bubbles: true }))
  viewport.scrollTop = viewport.scrollTop - 400
  viewport.dispatchEvent(new Event('scroll'))
  await frame()
  const top = viewport.scrollTop
  await append()
  await expect(Math.abs(viewport.scrollTop - top), 'a scroll up kept following').toBeLessThanOrEqual(1)
  await userEvent.click(canvas.getByRole('button', { name: 'New messages ↓' }))
  await framesSpanning(motionNumbers.standard)
  await expect(distance()).toBeLessThanOrEqual(1)
  await append()
  await expect(distance(), 'the jump control did not resume following').toBeLessThanOrEqual(1)
} }
export const FollowsTheLiveEndLight: Story = { ...FollowsTheLiveEnd, args: { scheme: 'light' } }
export const AllStates: Story = { render: args => <ScrollStory {...args} /> }
export const AllStatesLight: Story = { ...AllStates, args: { scheme: 'light' } }
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', height: '100vh', boxSizing: 'border-box', padding: s.lg, gap: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans }, actions: { display: 'flex', gap: s.md },
  button: { minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  frame: { display: 'flex', flex: '1 1 0', minHeight: 0, width: '100%' }, narrow: { width: '50%' }, viewport: { flex: '1 1 0', minHeight: 0, overflowY: 'auto', overflowAnchor: 'none' }, rows: { display: 'flex', flexDirection: 'column', gap: s.lg }, streamed: { marginBlock: 0, marginTop: s.md, fontFamily: t.fontMono, fontSize: t.metaSize, whiteSpace: 'pre-wrap' },
  row: { padding: s.lg, minHeight: g.resourceCard, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, fontSize: t.bodySize, lineHeight: t.bodyLeading },
})
