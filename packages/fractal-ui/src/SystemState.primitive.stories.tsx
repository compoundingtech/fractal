import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { SystemNotice } from './assistant-ui/composition/SystemState'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { darkTheme, liveAccentTheme } from './assistant-ui/embrace-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

function NoticeLane({ scheme }: { scheme: Scheme }) {
  return <main aria-label="Lane" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'dark' && darkTheme, liveAccentTheme, scheme === 'light' && lightTheme)}>
    <div style={{ width: 240 }}>
      <SystemNotice scope="inline" kind="error" title="Threads could not load" detail="The last read did not answer before the connection closed" onRetry={() => {}} />
    </div>
  </main>
}

// Keep data-independent notice geometry separate from the roster-backed system stories.
const meta = {
  title: 'Fractal UI/System states',
  excludeStories: ['scenarioExemptions'],
  component: NoticeLane,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'Inline notice text wrapping and Retry placement in a narrow lane; no domain data.' } },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<{ scheme: Scheme }>
export default meta
export const scenarioExemptions = ['InlineNoticeWraps:primitive'] as const
type Story = StoryObj<{ scheme: Scheme }>

/** An inline row in a narrow lane: the statement wraps in full and Retry follows it, never pushed out of view. */
export const InlineNoticeWraps: Story = {
  play: async ({ canvasElement }) => {
    const row = canvasElement.querySelector<HTMLElement>('[data-system-state="error"]')!
    const statement = row.querySelector<HTMLElement>('[role="status"]')!
    const retry = within(row).getByRole('button', { name: 'Retry: Threads could not load' })
    await expect(statement).toHaveTextContent('The last read did not answer before the connection closed')
    await expect(statement.scrollWidth).toBeLessThanOrEqual(statement.clientWidth + 1)
    await expect(retry.getBoundingClientRect().right).toBeLessThanOrEqual(row.getBoundingClientRect().right + 1)
    await expect(statement.compareDocumentPosition(retry) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  },
}

const styles = stylex.create({
  root: { height: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
})
