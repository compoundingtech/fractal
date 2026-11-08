import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { surfaceVars as surface, textVars as text, typeVars as type, spaceVars as space, borderVars as border } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { DiffView } from './assistant-ui/composition/DiffView'
import { samplePatch } from './assistant-ui/composition/diff-fixtures'



function shadowText(root: HTMLElement) {
  return [...root.querySelectorAll<HTMLElement>('[data-diff-file] *')].map(node => node.shadowRoot?.textContent ?? '').join('')
}

function Canvas({ scheme = 'dark', width = 1280, children }: { scheme?: 'dark' | 'light'; width?: number; children: React.ReactNode }) {
  return <div data-scheme={scheme} {...stylex.props(styles.canvas, ...(scheme === 'light' ? lightTheme : []))}><div {...stylex.props(styles.frame(width))}>{children}</div></div>
}

interface Args { scheme: 'dark' | 'light'; width: number; layout: 'unified' | 'split'; wrap: boolean; header: boolean; maxLines?: number; patch: string; path?: string }
function DiffViewStory({ scheme, width, ...props }: Args) {
  return <Canvas scheme={scheme} width={width}><DiffView {...props} /></Canvas>
}

const meta = {
  id: 'fractal-ui-diff-view',
  title: 'Kit/DiffView',
  component: DiffViewStory,
  args: { scheme: 'dark', width: 1280, layout: 'unified', wrap: false, header: true, patch: samplePatch },
  argTypes: { scheme: { control: 'inline-radio', options: ['dark', 'light'] }, layout: { control: 'inline-radio', options: ['unified', 'split'] }, width: { control: 'inline-radio', options: [1280, 440] } },
  parameters: { scenario: { _tag: 'primitive', reason: 'The kit diff renderer over fixed patch text; no domain state.' }, layout: 'fullscreen' },
} satisfies Meta<typeof DiffViewStory>
export default meta
type Story = StoryObj<typeof meta>

const rendered = async (canvasElement: HTMLElement) => {
  const view = canvasElement.querySelector<HTMLElement>('[data-testid="diff-view"]')!
  await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
  await waitFor(() => expect(shadowText(view)).toContain('retained row'), { timeout: 10_000 })
  return view
}

export const Unified: Story = {
  play: async ({ canvasElement }) => {
    const view = await rendered(canvasElement)
    await expect(view.dataset.added).toBe('3')
    await expect(view.dataset.removed).toBe('2')
    await expect(view.getAttribute('role')).toBe('region')
    await expect(view.tabIndex).toBe(0)
  },
}
export const UnifiedLight: Story = { args: { scheme: 'light' }, play: Unified.play }
export const Split: Story = { args: { layout: 'split' }, play: Unified.play }
export const SplitLight: Story = { args: { layout: 'split', scheme: 'light' }, play: Unified.play }
export const Narrow: Story = { args: { width: 440, wrap: true }, play: Unified.play }
export const NarrowLight: Story = { args: { width: 440, wrap: true, scheme: 'light' }, play: Unified.play }

/** Markdown ```diff fences: bare lines, no header, synthesized offsets render without line numbers. */
export const BareFence: Story = {
  args: { header: false, path: 'notes.md', patch: ' keep this\n-drop this\n+add this\n+and this' },
  play: async ({ canvasElement }) => {
    const view = canvasElement.querySelector<HTMLElement>('[data-testid="diff-view"]')!
    await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
    await expect(view.dataset.added).toBe('2')
    await expect(view.dataset.removed).toBe('1')
  },
}

/** Hover-card preview: the first six lines and a count of the rest. */
export const Compact: Story = {
  args: { header: false, maxLines: 6, width: 440 },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector('[data-diff-more]')?.textContent).toBe('6 more lines')
  },
}

/** Streaming: appending lines never shrinks the region, and the last stable render stays up. */
export const Streaming: Story = {
  args: { header: false },
  render: args => {
    const lines = samplePatch.split('\n')
    const [count, setCount] = React.useState(7)
    React.useEffect(() => {
      if (count >= lines.length) return
      const id = setTimeout(() => setCount(value => value + 1), 60)
      return () => clearTimeout(id)
    }, [count, lines.length])
    return <DiffViewStory {...args} patch={lines.slice(0, count).join('\n')} />
  },
  play: async ({ canvasElement }) => {
    const view = canvasElement.querySelector<HTMLElement>('[data-testid="diff-view"]')!
    let last = 0
    let shrank = false
    const observer = new ResizeObserver(() => { const height = view.getBoundingClientRect().height; if (height + 1 < last) shrank = true; last = Math.max(last, height) })
    observer.observe(view)
    await waitFor(() => expect(shadowText(view)).toContain('strict: true'), { timeout: 10_000 })
    observer.disconnect()
    await expect(shrank).toBe(false)
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, padding: space.xl, boxSizing: 'border-box' },
  frame: (width: number) => ({ width, maxWidth: '100%', borderWidth: 1, borderStyle: 'solid', borderColor: border.border }),
})
