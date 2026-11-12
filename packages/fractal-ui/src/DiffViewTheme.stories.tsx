import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { surfaceVars as surface, textVars as text, typeVars as type, spaceVars as space, borderVars as border } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { DiffView } from './assistant-ui/composition/DiffView'
import { Markdown } from './assistant-ui/composition/Markdown'
import { markdownBlock } from './assistant-ui/composition/markdown-text'
import { samplePatch } from './assistant-ui/composition/diff-sample'

const code = [
  '```ts',
  '// Rows the table keeps between renders.',
  'import { retain } from "./retain"',
  '',
  'export const rows: readonly Row[] = [',
  '  [RETAINED, 2, true, "retained row"],',
  '  [ADDED, 3, null, `template ${name}`],',
  ']',
  '',
  'export function keep(row: Row) {',
  '  return retain(row, { strict: true })',
  '}',
  '```',
].join('\n')

/** The chosen palette (Pierre default) on a Markdown code block and a DiffView, side by side. */
function ThemeBoard({ scheme, width }: Args) {
  return <div data-scheme={scheme} {...stylex.props(styles.canvas, ...(scheme === 'light' ? lightTheme : []))}>
    <section data-syntax-theme="pierre" aria-label={`Pierre default, ${scheme}`} {...stylex.props(styles.frame(width))}>
      <h2 {...stylex.props(styles.heading)}>Pierre default · {scheme}</h2>
      <div {...stylex.props(styles.pair, width < 800 && styles.stack)}>
        <div {...stylex.props(styles.cell)}><Markdown text={markdownBlock(code)} /></div>
        <div {...stylex.props(styles.cell, styles.bordered)}><DiffView patch={samplePatch} header label={`Diff, Pierre default, ${scheme}`} /></div>
      </div>
    </section>
  </div>
}
interface Args { scheme: 'dark' | 'light'; width: number }

const meta = {
  id: 'fractal-ui-diff-view-theme',
  title: 'Kit/DiffView/Theme',
  component: ThemeBoard,
  excludeStories: ['scenarioExemptions'],
  args: { scheme: 'dark', width: 1280 },
  argTypes: { scheme: { control: 'inline-radio', options: ['dark', 'light'] }, width: { control: 'inline-radio', options: [1280, 440] } },
  parameters: {
    scenario: { _tag: 'primitive', reason: 'The shared syntax palette on code blocks and diffs over fixed source; no domain state.' },
    layout: 'fullscreen',
    docs: { description: { component: 'Reference for the one syntax palette (Pierre default, light and dark): Markdown code blocks and the kit DiffView (@pierre/diffs) both read syntaxVars.' } },
  },
} satisfies Meta<typeof ThemeBoard>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark:primitive',
  'DarkNarrow:primitive',
  'Light:primitive',
  'LightNarrow:primitive',
]
type Story = StoryObj<typeof meta>

/** Code block and diff paint the same keyword colour: one palette, two renderers. */
const sharedPalette: Story['play'] = async ({ canvasElement }) => {
  for (const section of canvasElement.querySelectorAll<HTMLElement>('[data-syntax-theme]')) {
    const view = section.querySelector<HTMLElement>('[data-testid="diff-view"]')!
    await waitFor(() => expect(view.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
    const keyword = await waitFor(() => { const token = section.querySelector<HTMLElement>('[data-syntax-token="keyword"]'); expect(token).not.toBeNull(); return token! }, { timeout: 10_000 })
    const variable = getComputedStyle(view).getPropertyValue('--fractal-diff-token-token-keyword').trim()
    const probe = document.createElement('span')
    probe.style.color = variable
    section.append(probe)
    await expect(getComputedStyle(probe).color).toBe(getComputedStyle(keyword).color)
    probe.remove()
  }
}

export const Dark: Story = { play: sharedPalette }
export const Light: Story = { args: { scheme: 'light' }, play: sharedPalette }
export const DarkNarrow: Story = { args: { width: 440 }, play: sharedPalette }
export const LightNarrow: Story = { args: { scheme: 'light', width: 440 }, play: sharedPalette }

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, padding: space.xl, boxSizing: 'border-box' },
  frame: (width: number) => ({ width, maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: space.md }),
  heading: { margin: 0, fontSize: type.metaSize, fontWeight: type.weightMedium, color: text.fgMuted },
  pair: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 1fr)', gap: space.lg, alignItems: 'start' },
  stack: { gridTemplateColumns: 'minmax(0, 1fr)' },
  cell: { minWidth: 0 },
  bordered: { borderWidth: 1, borderStyle: 'solid', borderColor: border.border, overflow: 'hidden' },
})
