import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { surfaceVars as surface, textVars as text, typeVars as type, spaceVars as space, borderVars as border } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { DiffView } from './assistant-ui/composition/DiffView'
import { samplePatch } from './assistant-ui/composition/diff-fixtures'
import { EmbraceDiffPreview } from './assistant-ui/EmbraceToolPreview'
import { ResourceEntityCard } from './assistant-ui/entity/ResourceCard'
import { toolDiffs } from './assistant-ui/embrace-tool-preview'



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
  excludeStories: ['scenarioExemptions'],
  args: { scheme: 'dark', width: 1280, layout: 'unified', wrap: false, header: true, patch: samplePatch },
  argTypes: { scheme: { control: 'inline-radio', options: ['dark', 'light'] }, layout: { control: 'inline-radio', options: ['unified', 'split'] }, width: { control: 'inline-radio', options: [1280, 440] } },
  parameters: { scenario: { _tag: 'primitive', reason: 'The kit diff renderer over fixed patch text; no domain state.' }, layout: 'fullscreen' },
} satisfies Meta<typeof DiffViewStory>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'BareFence:primitive',
  'Compact:primitive',
  'Narrow:primitive',
  'NarrowLight:primitive',
  'Split:primitive',
  'SplitLight:primitive',
  'Streaming:primitive',
  'Unified:primitive',
  'UnifiedLight:primitive',
]
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

/** Transcript Edit/Write results: one DiffView per file under a caption with path and +N −M. */
export const TranscriptToolDiff: Story = {
  render: ({ scheme, width }) => <Canvas scheme={scheme} width={width}><EmbraceDiffPreview diffs={[
    ...toolDiffs({ file_path: 'src/rows.ts', old_string: 'export const rows = [\n  [PREVIOUS, 1]\n]', new_string: 'export const rows = [\n  [RETAINED, 2],\n  [ADDED, 3]\n]' }, undefined),
    ...toolDiffs({ patch: samplePatch }, undefined),
  ]} /></Canvas>,
  play: async ({ canvasElement }) => {
    const figures = [...canvasElement.querySelectorAll('figure')]
    await expect(figures.length).toBe(2)
    for (const figure of figures) {
      const view = figure.querySelector<HTMLElement>('[data-testid="diff-view"]')!
      await expect(view).not.toBeNull()
      await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
      await expect(figure.querySelector('figcaption')?.textContent).toContain(`+${view.dataset.added} −${view.dataset.removed}`)
    }
    await expect(figures[0]!.querySelector('figcaption')?.textContent).toContain('excerpt')
  },
}
export const TranscriptToolDiffLightNarrow: Story = { args: { scheme: 'light', width: 440 }, render: TranscriptToolDiff.render, play: TranscriptToolDiff.play }

/** Resource hover cards: a compact six-line DiffView instead of tinted text lines. */
export const HoverCardPreview: Story = {
  render: ({ scheme }) => <Canvas scheme={scheme} width={360}><ResourceEntityCard id="src/rows.ts" title="src/rows.ts" kind="Diff" icon="swap" summary={['+3 −2']} previewKind="diff" preview={samplePatch.split('\n').slice(4)} /></Canvas>,
  play: async ({ canvasElement }) => {
    const view = canvasElement.querySelector<HTMLElement>('[data-entity-card-preview="diff"] [data-testid="diff-view"]')!
    await expect(view).not.toBeNull()
    await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
    await expect(view.dataset.density).toBe('compact')
    await expect(canvasElement.querySelector('[data-diff-more]')?.textContent).toBe('6 more lines')
  },
}
export const HoverCardPreviewLight: Story = { args: { scheme: 'light' }, render: HoverCardPreview.render, play: HoverCardPreview.play }

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, padding: space.xl, boxSizing: 'border-box' },
  frame: (width: number) => ({ width, maxWidth: '100%', borderWidth: 1, borderStyle: 'solid', borderColor: border.border }),
})
