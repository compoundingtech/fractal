import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect } from 'storybook/test'
import { SidebarStatus } from './SidebarStatus'
import type { AgentStatus } from './model'
import { spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

const storyNow = Date.UTC(2026, 9, 8, 12, 0, 0)
const statuses: readonly AgentStatus[] = ['working', 'waiting', 'idle']
/** The same last observed status, once live and once retained while the host reconnects. */
function RetainedStatus() {
  return <main {...stylex.props(styles.root)}>{(['live', 'stale'] as const).map(freshness => <section key={freshness} data-status-freshness={freshness} {...stylex.props(styles.column)}>
    {statuses.map(status => <SidebarStatus key={status} status={status} statusSince={storyNow - 90_000} now={storyNow} freshness={freshness} />)}
  </section>)}</main>
}
const meta = { title: 'Fractal UI/Sidebar/Status', component: RetainedStatus, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof RetainedStatus>
export default meta
type Story = StoryObj<typeof meta>

export const RetainedWhileReconnecting: Story = { play: async ({ canvasElement }) => {
  const glyphs = (freshness: string) => [...canvasElement.querySelectorAll<HTMLElement>(`[data-status-freshness="${freshness}"] [data-observed-status]`)]
  const live = glyphs('live')
  const retained = glyphs('stale')
  // The retained glyph keeps the last observed status and says it is stale, rather than dropping to an unobserved state.
  await expect(retained.map(glyph => glyph.dataset.observedStatus)).toEqual(statuses)
  await expect(retained.every(glyph => glyph.dataset.freshness === 'stale')).toBe(true)
  await expect(retained[0]!.querySelector('[role="img"]')).toHaveAccessibleName(/stale observation/)
  const assertDimmed = async (glyph: HTMLElement) => { await expect(Number(getComputedStyle(glyph).opacity)).toBeLessThan(1) }
  for (const glyph of retained) await assertDimmed(glyph)
  // Negative control: a live glyph must fail the retained presentation assertion.
  await expect(Number(getComputedStyle(live[0]!).opacity)).toBe(1)
  await expect(assertDimmed(live[0]!)).rejects.toThrow()
} }

const styles = stylex.create({
  root: { display: 'flex', gap: s.xl, padding: s.lg, minHeight: '100vh', boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  column: { display: 'flex', flexDirection: 'column', gap: s.md },
})
