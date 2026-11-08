import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { surfaceVars as surface, textVars as text, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { DiffFiles, type DiffFilesEntry } from './assistant-ui/composition/DiffFiles'
import { samplePatch } from './assistant-ui/composition/diff-sample'

const body = (lines: number, prefix: string) => Array.from({ length: lines }, (_, index) => ` ${prefix} context ${index + 1}`)
const pullRequestFiles: readonly DiffFilesEntry[] = [
  { path: 'src/rows.ts', patch: samplePatch, added: 3, removed: 2, status: 'modified' },
  { path: 'src/scheduler/beds.rs', patch: ['@@ -1,0 +1,4 @@', '+pub struct Bed {', '+    pub id: u32,', '+    pub rows: Vec<Row>,', '+}'], added: 4, removed: 0, status: 'added' },
  { path: 'src/scheduler/mod.rs', patch: ['@@ -10,24 +10,24 @@', ...body(10, 'fn'), '-    let beds = load();', '+    let beds = load_beds()?;', ...body(12, 'fn')], added: 1, removed: 1, status: 'modified' },
  { path: 'docs/garden.md', patch: ['@@ -1,3 +0,0 @@', '-# Garden', '-', '-Old notes.'], added: 0, removed: 3, status: 'deleted' },
  { path: 'web/app/Planner.tsx', patch: ['@@ -4,3 +4,3 @@', ' export function Planner() {', '-  return <Beds />', '+  return <Beds layout="grid" />', ' }'], added: 1, removed: 1, status: 'modified' },
]

interface Args { scheme: 'dark' | 'light'; width: number; height: number }
function DiffFilesStory({ scheme, width, height }: Args) {
  return <div data-scheme={scheme} {...stylex.props(styles.canvas, ...(scheme === 'light' ? lightTheme : []))}>
    <div {...stylex.props(styles.frame(width, height))}><DiffFiles files={pullRequestFiles} label="Files changed, acme/garden#7" /></div>
  </div>
}

const meta = {
  id: 'fractal-ui-diff-files',
  title: 'Kit/DiffFiles',
  component: DiffFilesStory,
  args: { scheme: 'dark', width: 1280, height: 760 },
  argTypes: { scheme: { control: 'inline-radio', options: ['dark', 'light'] }, width: { control: 'inline-radio', options: [1280, 440] } },
  parameters: { scenario: { _tag: 'primitive', reason: 'Multi-file diff viewer over fixed patches; no domain state.' }, layout: 'fullscreen' },
} satisfies Meta<typeof DiffFilesStory>
export default meta
type Story = StoryObj<typeof meta>

const treeItems = (canvasElement: HTMLElement) => [...canvasElement.querySelectorAll('[data-testid="diff-files-tree"] *')].flatMap(node => [...(node.shadowRoot?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? [])])
const treeText = (canvasElement: HTMLElement) => treeItems(canvasElement).map(item => `${item.dataset.itemPath} ${item.textContent}`).join('\n')
const focusedSection = () => (document.activeElement as HTMLElement | null)?.closest<HTMLElement>('[data-diff-files-section]')?.dataset.diffFilesSection

const wide: Story['play'] = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const viewer = canvas.getByRole('region', { name: 'Files changed, acme/garden#7' })
  await expect(within(viewer).getByRole('toolbar', { name: 'Diff options' })).toBeVisible()
  // diffshub: a @pierre/trees navigator with every file and its +/− counts.
  await waitFor(() => expect(treeText(canvasElement)).toContain('beds.rs'), { timeout: 10_000 })
  await expect(treeText(canvasElement)).toContain('+4 −0')
  // Selecting a file in the tree jumps to it and focuses its header.
  await userEvent.click(treeItems(canvasElement).find(item => item.dataset.itemPath === 'web/app/Planner.tsx')!)
  await waitFor(() => expect(focusedSection()).toBe('web/app/Planner.tsx'))
  // Per-file headers carry the counts; the first file renders its diff.
  const first = canvasElement.querySelector<HTMLElement>('[data-diff-files-section="src/rows.ts"]')!
  await expect(within(first).getByLabelText('3 added, 2 removed')).toBeVisible()
  await waitFor(() => expect(first.querySelector<HTMLElement>('[data-testid="diff-view"]')?.dataset.diffState).toBe('rendered'), { timeout: 10_000 })
  // j/k jump between files and move focus to that file's header.
  within(first).getByRole('button', { name: 'src/rows.ts' }).focus()
  await userEvent.keyboard('j')
  await expect(focusedSection()).toBe('src/scheduler/beds.rs')
  await userEvent.keyboard('jj')
  await expect(focusedSection()).toBe('docs/garden.md')
  await userEvent.keyboard('k')
  await expect(focusedSection()).toBe('src/scheduler/mod.rs')
  // Collapse per file.
  const toggle = within(canvasElement.querySelector<HTMLElement>('[data-diff-files-section="src/scheduler/mod.rs"]')!).getByRole('button', { name: 'src/scheduler/mod.rs' })
  await userEvent.click(toggle)
  await expect(toggle).toHaveAttribute('aria-expanded', 'false')
  await userEvent.click(toggle)
  await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  // Split or unified.
  await userEvent.click(canvas.getByRole('button', { name: 'Split' }))
  await expect(viewer.dataset.layout).toBe('split')
  await userEvent.click(canvas.getByRole('button', { name: 'Collapse all' }))
  await expect(canvasElement.querySelectorAll('[data-diff-files-toggle][aria-expanded="true"]').length).toBe(0)
}

export const Dark: Story = { play: wide }
export const Light: Story = { args: { scheme: 'light' }, play: wide }

/** 440px: the tree starts hidden behind its toggle; files keep their headers and counts. */
const narrow: Story['play'] = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const viewer = canvas.getByRole('region', { name: 'Files changed, acme/garden#7' })
  await waitFor(() => expect(viewer.dataset.tree).toBe('false'))
  await expect(canvasElement.querySelector('[data-testid="diff-files-tree"]')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'File tree' }))
  await waitFor(() => expect(treeText(canvasElement)).toContain('Planner.tsx'), { timeout: 10_000 })
  await expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(document.documentElement.clientWidth)
}
export const Narrow: Story = { args: { width: 440 }, play: narrow }
export const NarrowLight: Story = { args: { width: 440, scheme: 'light' }, play: narrow }

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: text.fg, fontFamily: type.fontSans, boxSizing: 'border-box' },
  frame: (width: number, height: number) => ({ width, maxWidth: '100%', height }),
})
