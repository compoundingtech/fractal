import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { surfaceVars as surface, textVars as text, typeVars as type, spaceVars as space, borderVars as border } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { DiffView } from './assistant-ui/composition/DiffView'
import { samplePatch } from './assistant-ui/composition/diff-sample'
import { EmbraceDiffPreview } from './assistant-ui/EmbraceToolPreview'
import { DiffPanel } from './assistant-ui/composition/DiffPanel'
import { userEvent } from 'storybook/test'
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
  'ChangesPanel:primitive',
  'ChangesPanelLightNarrow:primitive',
  'Compact:primitive',
  'HoverCardPreview:primitive',
  'HoverCardPreviewLight:primitive',
  'Narrow:primitive',
  'NarrowLight:primitive',
  'SchemeSwitch:primitive',
  'Split:primitive',
  'SplitLight:primitive',
  'Streaming:primitive',
  'TranscriptToolDiff:primitive',
  'TranscriptToolDiffLightNarrow:primitive',
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
/** A scrolling split column takes keyboard focus and draws the shared focus ring the host passes into the shadow root. */
export const SplitFocusRing: Story = {
  args: { layout: 'split' },
  play: async ({ canvasElement }) => {
    const view = await rendered(canvasElement)
    const host = [...view.querySelectorAll<HTMLElement>('[data-diff-file] *')].find(node => node.shadowRoot !== null)!
    const code = host.shadowRoot!.querySelector<HTMLElement>('[data-code]')!
    await waitFor(() => expect(code.tabIndex).toBe(0))
    code.focus()
    await expect(code.matches(':focus-visible')).toBe(true)
    // Resolve the shared ring tokens in the host's context, the values the shadow CSS must draw.
    const probe = canvasElement.ownerDocument.createElement('div')
    probe.style.cssText = 'outline-style: solid; outline-width: var(--fractal-diff-focus-width); outline-color: var(--fractal-diff-focus-color); outline-offset: var(--fractal-diff-focus-inset)'
    view.append(probe)
    try {
      const ring = getComputedStyle(code), shared = getComputedStyle(probe)
      await expect(ring.outlineStyle).toBe('solid')
      await expect(ring.outlineWidth).toBe(shared.outlineWidth)
      await expect(ring.outlineColor).toBe(shared.outlineColor)
      await expect(ring.outlineOffset).toBe(shared.outlineOffset)
    } finally { probe.remove() }
  },
}
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

/** The Changes panel: each file's body is a DiffView; the toolbar's layout, wrap and whitespace toggles drive it. */
export const ChangesPanel: Story = {
  render: ({ scheme, width }) => <Canvas scheme={scheme} width={width}><div style={{ height: 640, display: 'flex' }}><DiffPanel open title="Changes" showSurfaceTabs={false} branchFiles={[
    { path: 'src/rows.ts', diff: samplePatch.split('\n').slice(4), added: 3, removed: 2 },
    { path: 'src/space.ts', diff: ['@@ -1,2 +1,2 @@', '-const  a = 1', '+const a = 1', ' const b = 2'], added: 1, removed: 1 },
  ]} /></div></Canvas>,
  play: async ({ canvasElement }) => {
    const views = () => [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="diff-code"]')]
    await expect(views().length).toBe(2)
    for (const view of views()) await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
    await expect(views()[0]!.getAttribute('aria-label')).toBe('Diff lines for src/rows.ts')
    await userEvent.click(canvasElement.querySelector<HTMLElement>('[aria-label="Hide whitespace-only changes"]')!)
    await waitFor(() => expect(views()[1]!.dataset.added).toBe('0'))
    await expect(canvasElement.textContent).toContain('No non-whitespace changes.')
  },
}
export const ChangesPanelLightNarrow: Story = { args: { scheme: 'light', width: 440 }, render: ChangesPanel.render, play: ChangesPanel.play }

/** Resource hover cards: a compact six-line DiffView instead of tinted text lines. */
export const HoverCardPreview: Story = {
  render: ({ scheme }) => <Canvas scheme={scheme} width={360}><ResourceEntityCard id="src/rows.ts" title="src/rows.ts" kind="Diff" icon="swap" summary={['+3 −2']} previewKind="diff" preview={samplePatch.split('\n').slice(4)} /></Canvas>,
  play: async ({ canvasElement }) => {
    const view = canvasElement.querySelector<HTMLElement>('[data-entity-card-preview="diff"] [data-testid="diff-view"]')!
    await expect(view).not.toBeNull()
    await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
    await expect(view.dataset.density).toBe('compact')
    await expect(canvasElement.querySelector('[data-diff-more]')?.textContent).toBe('6 more lines')
    // The card is a pointer-events:none tooltip: nothing inside it may take focus.
    await expect(view.hasAttribute('tabindex')).toBe(false)
    const shadow = [...view.querySelectorAll('*')].find(element => element.shadowRoot)!.shadowRoot!
    await expect(shadow.querySelectorAll('button, a[href], [tabindex]:not([tabindex="-1"])').length).toBe(0)
  },
}
export const HoverCardPreviewLight: Story = { args: { scheme: 'light' }, render: HoverCardPreview.render, play: HoverCardPreview.play }

/** A live theme switch (no remount) moves Pierre's color-scheme with the surface. */
export const SchemeSwitch: Story = {
  render: args => {
    const [scheme, setScheme] = React.useState<'dark' | 'light'>('dark')
    return <div>
      <button type="button" onClick={() => setScheme(value => value === 'dark' ? 'light' : 'dark')}>Switch scheme</button>
      <Canvas scheme={scheme} width={args.width}><DiffView patch={args.patch} header /></Canvas>
    </div>
  },
  play: async ({ canvasElement }) => {
    const view = await rendered(canvasElement)
    const pierreHost = () => [...view.querySelectorAll<HTMLElement>('*')].find(element => element.shadowRoot)!
    await expect(getComputedStyle(pierreHost()).colorScheme).toBe('dark')
    await userEvent.click(canvasElement.querySelector('button')!)
    await waitFor(() => expect(getComputedStyle(pierreHost()).colorScheme).toBe('light'), { timeout: 5_000 })
    await userEvent.click(canvasElement.querySelector('button')!)
    await waitFor(() => expect(getComputedStyle(pierreHost()).colorScheme).toBe('dark'), { timeout: 5_000 })
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, padding: space.xl, boxSizing: 'border-box' },
  frame: (width: number) => ({ width, maxWidth: '100%', borderWidth: 1, borderStyle: 'solid', borderColor: border.border }),
})
