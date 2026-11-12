import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { surfaceVars as surface, textVars as text, typeVars as type, spaceVars as space, borderVars as border } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { DiffView } from './assistant-ui/composition/DiffView'
import { Markdown } from './assistant-ui/composition/Markdown'
import { markdownBlock } from './assistant-ui/composition/markdown-text'
import { syntaxThemes, type SyntaxThemeName } from './assistant-ui/composition/syntax-themes'
import { samplePatch } from './assistant-ui/composition/diff-fixtures'

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

const names = Object.keys(syntaxThemes) as SyntaxThemeName[]

function Candidate({ name, scheme, width }: { name: SyntaxThemeName; scheme: 'dark' | 'light'; width: number }) {
  const theme = syntaxThemes[name][scheme]
  return <section data-syntax-theme={name} aria-label={`${syntaxThemes[name].label}, ${scheme}`} {...stylex.props(styles.candidate, theme)}>
    <h2 {...stylex.props(styles.heading)}>{syntaxThemes[name].label} · {scheme}</h2>
    <div {...stylex.props(styles.pair, width < 800 && styles.stack)}>
      <div {...stylex.props(styles.cell)}><Markdown text={markdownBlock(code)} /></div>
      <div {...stylex.props(styles.cell, styles.bordered)}><DiffView patch={samplePatch} header label={`Diff, ${syntaxThemes[name].label}, ${scheme}`} /></div>
    </div>
  </section>
}

interface Args { scheme: 'dark' | 'light'; width: number; candidate: SyntaxThemeName | 'all' }
function ThemeBoard({ scheme, width, candidate }: Args) {
  return <div data-scheme={scheme} {...stylex.props(styles.canvas, ...(scheme === 'light' ? lightTheme : []))}>
    <div {...stylex.props(styles.frame(width))}>{(candidate === 'all' ? names : [candidate]).map(name => <Candidate key={name} name={name} scheme={scheme} width={width} />)}</div>
  </div>
}

const meta = {
  id: 'fractal-ui-diff-view-theme',
  title: 'Kit/DiffView/Theme',
  component: ThemeBoard,
  args: { scheme: 'dark', width: 1280, candidate: 'all' },
  argTypes: { scheme: { control: 'inline-radio', options: ['dark', 'light'] }, width: { control: 'inline-radio', options: [1280, 440] }, candidate: { control: 'inline-radio', options: ['all', ...names] } },
  parameters: {
    scenario: { _tag: 'primitive', reason: 'Palette candidates for code blocks and diffs over fixed source; no domain state.' },
    layout: 'fullscreen',
    docs: { description: { component: 'Pick one palette: Markdown code blocks and the kit DiffView (@pierre/diffs) read the same syntaxVars, so the chosen candidate applies to both.' } },
  },
} satisfies Meta<typeof ThemeBoard>
export default meta
type Story = StoryObj<typeof meta>

/** Code block and diff paint the same keyword colour in every candidate: one palette, two renderers. */
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
  frame: (width: number) => ({ width, maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: 32 }),
  candidate: { display: 'flex', flexDirection: 'column', gap: space.md },
  heading: { margin: 0, fontSize: type.metaSize, fontWeight: type.weightMedium, color: text.fgMuted },
  pair: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 1fr)', gap: space.lg, alignItems: 'start' },
  stack: { gridTemplateColumns: 'minmax(0, 1fr)' },
  cell: { minWidth: 0 },
  bordered: { borderWidth: 1, borderStyle: 'solid', borderColor: border.border, overflow: 'hidden' },
})
