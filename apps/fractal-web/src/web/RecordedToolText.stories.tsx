import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { RecordedToolText, toolPreviewLines } from './RecordedToolText.tsx'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { assistantDarkTheme, liveAccentTheme, liveComposerDarkTheme, compositionLightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/shell.ts'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

const recordedJson = JSON.stringify({ path: 'src/catalog.ts', content: Array.from({ length: 200 }, (_, index) => `export const row${index} = "a readable catalog row with several fields";`).join('\n'), complete: true })
const PreviewSurface = ({ scheme = 'dark', width = 1440, unclamped = false }: { readonly scheme?: 'dark' | 'light'; readonly width?: number; readonly unclamped?: boolean }) => <div data-recorded-preview-story="" data-unclamped={unclamped} {...stylex.props(liveLegacyTheme, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)} style={{ backgroundColor: c.canvas, color: c.fg, width: Math.min(width, window.innerWidth), padding: 24, boxSizing: 'border-box' }}>
  <section aria-label="Recorded result" style={{ width: '100%', maxWidth: 380 }}><RecordedToolText text={recordedJson} /></section>
</div>
const previewPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const root = canvasElement.querySelector<HTMLElement>('[data-recorded-preview-story]')!
  const result = within(root).getByRole('region', { name: 'Recorded result' })
  const preview = result.querySelector<HTMLElement>('[data-tool-output]')!
  await expect(preview.textContent).toContain('{\n  "path": "src/catalog.ts",\n  "content":')
  await expect(result.querySelector('[data-recorded-text]')).toHaveAttribute('data-recorded-lines', '1')
  if (root.dataset.unclamped === 'true') { preview.style.maxHeight = 'none'; preview.style.overflow = 'visible' }
  const lineHeight = Number.parseFloat(getComputedStyle(preview).lineHeight)
  await expect(lineHeight).toBeGreaterThan(0)
  await expect(preview.getBoundingClientRect().height).toBeLessThanOrEqual(toolPreviewLines * lineHeight + 1)
  const height = result.getBoundingClientRect().height
  await userEvent.click(within(result).getByRole('button', { name: 'Show all 5 lines' }))
  await expect(preview.textContent).toBe(JSON.stringify(JSON.parse(recordedJson), null, 2))
  await expect(result.getBoundingClientRect().height).toBe(height)
  await userEvent.click(within(result).getByRole('button', { name: 'Show 12-line preview' }))
  await expect(preview.getBoundingClientRect().height).toBeLessThanOrEqual(toolPreviewLines * lineHeight + 1)
  await expect(result.getBoundingClientRect().height).toBe(height)
  root.dataset.proof = 'passed'
}
const meta = { title: 'Fractal/App/Transcript/Recorded tool preview', component: PreviewSurface, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof PreviewSurface>
export default meta
type Story = StoryObj<typeof meta>
export const Dark1440: Story = { args: { scheme: 'dark', width: 1440 }, play: previewPlay }
export const Light1440: Story = { args: { scheme: 'light', width: 1440 }, play: previewPlay }
export const Dark900: Story = { args: { scheme: 'dark', width: 900 }, play: previewPlay }
export const Light900: Story = { args: { scheme: 'light', width: 900 }, play: previewPlay }
export const Dark440: Story = { args: { scheme: 'dark', width: 440 }, play: previewPlay }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play: previewPlay }
/** Same predicate and payload, with the physical preview bound deliberately removed. */
export const ExpectedFailureHarness: Story = { args: { width: 440, unclamped: true }, play: async context => {
  await expect(previewPlay(context)).rejects.toThrow()
} }
