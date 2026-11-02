import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { EmbraceScrollViewport } from './assistant-ui/EmbraceScrollViewport'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { interactionVars as interaction, surfaceVars as surface, textVars as ink, borderVars as border, spaceVars as s, typeVars as t, geometryVars as g } from './assistant-ui/composition-tokens.stylex'

function ScrollStory({ scheme = 'dark' }: { scheme?: Scheme }) {
  const [count, setCount] = React.useState(40)
  const [narrow, setNarrow] = React.useState(false)
  const rows = React.useMemo(() => Array.from({ length: count }, (_, index) => ({ id: `entry-${index}`, text: `Observed message ${index + 1}: preserve the reader's position as the conversation grows or the viewport changes width.` })), [count])
  return <main {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><div {...stylex.props(styles.actions)}><Button onPress={() => setCount(value => value + 1)} {...stylex.props(styles.button)}>Append message</Button><Button onPress={() => setNarrow(value => !value)} {...stylex.props(styles.button)}>Change width</Button></div><section {...stylex.props(styles.frame, narrow && styles.narrow)}><EmbraceScrollViewport items={rows} data-testid="scroll-viewport" aria-label="Conversation history" tabIndex={0} {...stylex.props(styles.viewport)} contentProps={stylex.props(styles.rows)}>{rows.map(row => <article key={row.id} data-item-id={row.id} {...stylex.props(styles.row)}>{row.text}</article>)}</EmbraceScrollViewport></section></main>
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
]
type Story = StoryObj<typeof meta>
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
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
  // The return eases over the standard motion duration (200ms), or lands at once under reduced motion.
  await waitFor(() => expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1), { timeout: 400 })
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
export const AllStates: Story = { render: args => <ScrollStory {...args} /> }
export const AllStatesLight: Story = { ...AllStates, args: { scheme: 'light' } }
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', height: '100vh', boxSizing: 'border-box', padding: s.lg, gap: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans }, actions: { display: 'flex', gap: s.md },
  button: { minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  frame: { display: 'flex', flex: '1 1 0', minHeight: 0, width: '100%' }, narrow: { width: '50%' }, viewport: { flex: '1 1 0', minHeight: 0, overflowY: 'auto', overflowAnchor: 'none' }, rows: { display: 'flex', flexDirection: 'column', gap: s.lg }, row: { padding: s.lg, minHeight: g.resourceCard, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, fontSize: t.bodySize, lineHeight: t.bodyLeading },
})
