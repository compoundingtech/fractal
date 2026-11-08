import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, interactionVars, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { SkipLink } from './SkipLink.tsx'

/** One conversation face in a pane; a retained hidden tab stays mounted, aria-hidden and inert, as in the workspace. */
const Surface = ({ tab, pane, shown = true }: { readonly tab: string; readonly pane: string; readonly shown?: boolean }) =>
  <div data-tab-surface-id={tab} data-surface-pane={pane} aria-hidden={!shown} inert={!shown} {...stylex.props(shown ? styles.surface : styles.hidden)}>
    <div data-wf-shown-pane={shown ? '' : undefined} {...stylex.props(styles.surface)}>
      <div data-testid="transcript-scroll" role="group" aria-label={`Conversation history ${tab}`} tabIndex={0} {...stylex.props(styles.lane)}>Messages in {tab}</div>
      <textarea aria-label={`Message ${tab}`} {...stylex.props(styles.composer)} />
    </div>
  </div>

/**
 * The app's first tab stop skips the devbar and global area to the focused pane's conversation: here the right
 * pane of a split, behind a left pane earlier in the DOM and a retained hidden tab.
 */
const SkipLinkSurface = ({ scheme = 'dark', width = 1280 }: { readonly scheme?: 'dark' | 'light'; readonly width?: number }) =>
  <main data-scheme={scheme} {...stylex.props(styles.root, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)} style={{ width }}>
    <span data-testid="ring-reference" aria-hidden="true" {...stylex.props(styles.reference)} />
    <SkipLink />
    <nav aria-label="Developer tools" {...stylex.props(styles.bar)}><button type="button">Dev tools</button><button type="button">Counters</button></nav>
    <aside aria-label="Agents" {...stylex.props(styles.bar)}><button type="button">Collapse agents</button></aside>
    <div {...stylex.props(styles.split, width <= 440 && styles.stacked)}>
      <section data-pane="left" aria-label="Left pane" {...stylex.props(styles.pane)}><Surface tab="t1" pane="left" /></section>
      <section data-pane="right" data-focused="" aria-label="Right pane" {...stylex.props(styles.pane)}><Surface tab="t0" pane="right" shown={false} /><Surface tab="t2" pane="right" /></section>
    </div>
  </main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Surfaces/Skip link',
  component: SkipLinkSurface,
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Pins the skip link\'s tab order, visibility and target over a stand-in devbar, global area and split.' }, layout: 'fullscreen' },
} satisfies Meta<typeof SkipLinkSurface>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark1280:contract-fixture',
  'Dark440:contract-fixture',
  'Light1280:contract-fixture',
  'Light440:contract-fixture',
]
type Story = StoryObj<typeof meta>

const play: Story['play'] = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const doc = canvasElement.ownerDocument
  const link = canvas.getByRole('link', { name: 'Skip to conversation' })
  // Hidden until focused: no box a sighted pointer user can see.
  await expect(link.getBoundingClientRect().width).toBeLessThanOrEqual(1)
  doc.body.focus()
  await userEvent.tab()
  await waitFor(() => expect(doc.activeElement).toBe(link))
  const shown = link.getBoundingClientRect()
  await expect(shown.width).toBeGreaterThan(40)
  await expect(shown.top).toBeGreaterThanOrEqual(0)
  await expect(getComputedStyle(link).outlineColor).toBe(getComputedStyle(canvas.getByTestId('ring-reference')).outlineColor)
  await expect(getComputedStyle(link).outlineWidth).toBe('1px')
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(doc.activeElement).toBe(canvas.getByRole('group', { name: 'Conversation history t2' })))
  // The next stop is the focused conversation's composer, not the devbar, the global area or the other pane.
  await userEvent.tab()
  await expect(doc.activeElement).toBe(canvas.getByRole('textbox', { name: 'Message t2' }))
  await waitFor(() => expect(link.getBoundingClientRect().width).toBeLessThanOrEqual(1))
}

export const Dark1280: Story = { args: { scheme: 'dark', width: 1280 }, play }
export const Light1280: Story = { args: { scheme: 'light', width: 1280 }, play }
export const Dark440: Story = { args: { scheme: 'dark', width: 440 }, play }
export const Light440: Story = { args: { scheme: 'light', width: 440 }, play }

const styles = stylex.create({
  root: { position: 'relative', minHeight: '100vh', maxWidth: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.md, padding: s.md, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  bar: { display: 'flex', gap: s.sm },
  split: { display: 'flex', gap: s.md },
  stacked: { flexDirection: 'column' },
  pane: { flex: '1 1 0', minWidth: 0, display: 'flex', flexDirection: 'column' },
  surface: { display: 'flex', flexDirection: 'column', gap: s.sm },
  hidden: { display: 'none' },
  lane: { minHeight: 120, padding: s.md },
  composer: { minHeight: 40 },
  reference: { position: 'absolute', visibility: 'hidden', outlineColor: interactionVars.focusRing },
})
