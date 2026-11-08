/** The GitHub tab as the host mounts it: `GithubSurface` reading through a `GithubReader` (here the fixture reader), per kind. Synthetic `acme/garden` data only. */
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../../ui-compat/live-theme.stylex.ts'
import { fixtureReader } from './card/GithubCard.stories.tsx'
import { GithubReaderProvider } from './data.ts'
import { parseSubjectRef } from './refs.ts'
import { GithubSurface } from './surface.tsx'

const Tab = ({ subjectRef }: { readonly subjectRef: string }) =>
  <main data-scheme="dark" {...stylex.props(styles.canvas, liveLegacyTheme, assistantDarkTheme)}>
    <GithubReaderProvider value={fixtureReader}>
      <div {...stylex.props(styles.tab)}><GithubSurface target={parseSubjectRef(subjectRef)!} /></div>
    </GithubReaderProvider>
  </main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Plugins/GitHub/Tab',
  component: Tab,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The fixture reader answers with the normative GitHub plugin wire fixtures, so the tab wiring is pinned without a daemon.' } },
} satisfies Meta<typeof Tab>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['CommentOnPullRequest:contract-fixture', 'PullRequest:contract-fixture', 'PullRequestNotFound:contract-fixture']

export const PullRequest: Story = {
  args: { subjectRef: 'resource/github/acme/garden/pull/7' },
  play: async ({ canvasElement }) => {
    const page = within(canvasElement)
    await expect(await page.findByText('Water the beds on a schedule', {}, { timeout: 2000 })).toBeVisible()
    await userEvent.click(page.getByRole('tab', { name: /Checks/ }))
    await expect(await page.findByText('test (ubuntu)', {}, { timeout: 2000 })).toBeVisible()
  },
}
export const PullRequestNotFound: Story = {
  args: { subjectRef: 'resource/github/acme/garden/pull-request/404' },
  play: async ({ canvasElement }) => { await expect(await within(canvasElement).findByText('Not found or no access', {}, { timeout: 2000 })).toBeVisible() },
}
export const CommentOnPullRequest: Story = {
  args: { subjectRef: 'resource/github/acme/garden/comment/1832451234' },
  // The comment sits on #7, which GitHub's issues read marks as a PR: the PR tab opens.
  play: async ({ canvasElement }) => { await expect(await within(canvasElement).findByText('Water the beds on a schedule', {}, { timeout: 2000 })).toBeVisible() },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: '24px', backgroundColor: surface.canvas, color: ink.fg },
  tab: { maxWidth: '880px' },
})
