/** The GitHub pull request tab (and the issue and run tabs in its shell) over the normative plugin fixtures. Synthetic `acme/garden` data only. */
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, fn, userEvent, within } from 'storybook/test'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../../../ui-compat/live-theme.stylex.ts'
import { checksRead, commentFixture, conversationRead, draftPullRequestFixture, filesRead, fixtureNow, goneRead, issueRead, movedRead, notFoundRead, pullRequestRead, runRead, stalePullRequestRead, unavailableRead } from '../fixtures.ts'
import type { GithubPullRequest, GithubRead } from '../schema.ts'
import type { LinkedThing } from './DetailShell.tsx'
import { IssueDetail } from './IssueDetail.tsx'
import { PullRequestDetail, type PullRequestSection } from './PullRequestDetail.tsx'
import { RunDetail } from './RunDetail.tsx'

const onOpen = fn()
const target = { kind: 'pull-request', owner: 'acme', repo: 'garden', number: 7 } as const
const linked: readonly LinkedThing[] = [
  { ref: 'agent/acme/gardener', label: 'gardener', kind: 'agent' },
  { ref: 'mission/water-schedule', label: 'Water schedule', kind: 'mission' },
]
const draftRead: GithubRead<GithubPullRequest> = { status: 'found', fetched_at: '2026-11-07T06:40:00Z', stale: false, item: draftPullRequestFixture }

const Canvas = ({ width = '880px', children }: { readonly width?: string; readonly children: React.ReactNode }) =>
  <main data-scheme="dark" {...stylex.props(styles.canvas, liveLegacyTheme, assistantDarkTheme)}>
    <div data-testid="tab-surface" {...stylex.props(styles.tab(width))}>{children}</div>
  </main>

interface SurfaceProps {
  readonly pullRequest: GithubRead<GithubPullRequest> | undefined
  readonly initial?: PullRequestSection
  readonly width?: string
  readonly number?: number
}

/** Holds the selected subtab the way the app's tab state will. */
const Surface = ({ pullRequest, initial = 'conversation', width, number = 7 }: SurfaceProps) => {
  const [section, setSection] = React.useState<PullRequestSection>(initial)
  return <Canvas width={width}>
    <PullRequestDetail target={{ ...target, number }} pullRequest={pullRequest} conversation={conversationRead} checks={checksRead} files={filesRead} linked={linked} now={fixtureNow} section={section} onSection={setSection} onOpen={onOpen} />
  </Canvas>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Plugins/GitHub/Pull request',
  component: Surface,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The normative GitHub plugin wire fixtures pin every read state, the subtabs and their link-outs.' } },
} satisfies Meta<typeof Surface>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Checks:contract-fixture', 'Conversation:contract-fixture', 'Draft:contract-fixture', 'Files:contract-fixture', 'Gone:contract-fixture', 'Issue:contract-fixture', 'Loading:contract-fixture', 'MergedStale:contract-fixture', 'Moved:contract-fixture', 'Narrow:contract-fixture', 'NotFound:contract-fixture', 'Run:contract-fixture', 'Unavailable:contract-fixture']

const githubLinks = (root: HTMLElement) => [...root.querySelectorAll<HTMLAnchorElement>('a[href]')]

const expectGithubLinksOnly = async (root: HTMLElement) => {
  for (const link of githubLinks(root)) {
    await expect(new URL(link.href).hostname).toBe('github.com')
    await expect(link.target).toBe('_blank')
    await expect(link.rel).toContain('noopener')
  }
  // People are logins with an initials disc; body images stay deferred links. Nothing remote loads.
  await expect(root.querySelector('img')).toBeNull()
}

export const Conversation: Story = {
  args: { pullRequest: pullRequestRead },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: 'Water the beds on a schedule #7' })).toBeVisible()
    await expect(canvas.getByText('Open', { selector: '[data-tone]' })).toBeVisible()
    await expect(canvas.getByText('Blocked')).toBeVisible()
    await expect(canvas.getByRole('link', { name: 'Open on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/pull/7')
    await expect(canvas.getByRole('tab', { name: 'Conversation 6' })).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByRole('heading', { name: 'Changes' })).toBeVisible()
    await expect(canvas.getByRole('article', { name: 'Review by grace: requested changes' })).toBeVisible()
    await expect(canvas.getByRole('article', { name: 'Review thread on src/scheduler.rs:42' })).toBeVisible()
    await expect(canvas.getByRole('article', { name: 'Review thread on README.md' })).toHaveTextContent('Outdated')
    await expect(canvas.getByTestId('deferred-image')).toHaveTextContent('Open image · github.com')
    const linkedNav = within(canvas.getByRole('navigation', { name: 'Linked' }))
    const buttons = linkedNav.getAllByRole('button')
    await expect(buttons.map(button => button.textContent)).toEqual(['Agent gardener', 'Mission Water schedule', 'Issue #9 Beds dry out over the weekend'])
    await userEvent.click(buttons[0]!)
    await expect(onOpen).toHaveBeenCalledWith('agent/acme/gardener')
    await userEvent.click(buttons[2]!)
    await expect(onOpen).toHaveBeenCalledWith('resource/github/acme/garden/issue/9')
    await expectGithubLinksOnly(canvasElement)
  },
}

export const Checks: Story = {
  args: { pullRequest: pullRequestRead },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('tab', { name: 'Conversation 6' }))
    await userEvent.keyboard('{ArrowRight}')
    await expect(canvas.getByRole('tab', { name: 'Checks 5' })).toHaveAttribute('aria-selected', 'true')
    const panel = within(canvas.getByRole('tabpanel'))
    await expect(panel.getByText('1 failed · 2 pending · 1 passed · 1 skipped or neutral')).toBeVisible()
    const rows = panel.getAllByRole('listitem')
    await expect(rows).toHaveLength(5)
    await expect(rows[1]).toHaveTextContent('Failed')
    await expect(rows[1]).toHaveTextContent('2 tests failed: scheduler::skips_dormant, scheduler::rain')
    await expect(rows[3]).toHaveTextContent('In progress')
    await expect(rows[3]).toHaveTextContent('44m')
    await expect(panel.getByRole('link', { name: 'Logs for test (ubuntu) on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/actions/runs/9001/job/2')
    await expectGithubLinksOnly(canvasElement)
  },
}

export const Files: Story = {
  args: { pullRequest: pullRequestRead, initial: 'files' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('tab', { name: 'Files 3' })).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByRole('complementary', { name: 'Files changed, acme/garden#7' })).toBeVisible()
    await expect(canvas.getAllByText('src/scheduler.rs').length).toBeGreaterThan(0)
    await expect(canvas.getByRole('link', { name: 'View assets/beds.png on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/pull/7/files')
    await expectGithubLinksOnly(canvasElement)
  },
}

export const Draft: Story = {
  args: { pullRequest: draftRead, number: 12 },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('Draft', { selector: '[data-tone]' })).toHaveAttribute('data-tone', 'draft')
    await expect(canvas.getByText('No description provided.')).toBeVisible()
  },
}

export const MergedStale: Story = {
  args: { pullRequest: stalePullRequestRead },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('Merged', { selector: '[data-tone]' })).toHaveAttribute('data-tone', 'merged')
    await expect(canvas.getByText('stale · fetched 5m ago')).toBeVisible()
    await expect(canvas.queryByText('Blocked')).toBeNull()
  },
}

export const Loading: Story = {
  args: { pullRequest: undefined },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('status')).toHaveTextContent('Loading pull request acme/garden#7')
    await expect(canvas.getByRole('link', { name: 'Open on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/pull/7')
  },
}

export const Moved: Story = {
  args: { pullRequest: movedRead },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('alert')).toHaveTextContent('Now at acme-garden/garden#7.')
    await expect(canvas.getByRole('link', { name: 'Open on GitHub' })).toHaveAttribute('href', 'https://github.com/acme-garden/garden/pull/7')
    await userEvent.click(canvas.getByRole('button', { name: 'Open acme-garden/garden#7' }))
    await expect(onOpen).toHaveBeenCalledWith('resource/github/acme-garden/garden/pull-request/7')
  },
}

export const Gone: Story = {
  args: { pullRequest: goneRead },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Deleted') },
}

export const NotFound: Story = {
  args: { pullRequest: notFoundRead },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('alert')).toHaveTextContent('Not found or no access')
    await expect(canvas.getByRole('link', { name: 'Open on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/pull/7')
  },
}

export const Unavailable: Story = {
  args: { pullRequest: unavailableRead },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByRole('alert')).toHaveTextContent('GitHub API rate limit exceeded for this installation Retry in 2m.') },
}

export const Issue: Story = {
  args: { pullRequest: undefined },
  render: () => <Canvas><IssueDetail target={{ kind: 'issue', owner: 'acme', repo: 'garden', number: 9 }} issue={issueRead} comments={[commentFixture]} linked={linked} now={fixtureNow} onOpen={onOpen} /></Canvas>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: 'Beds dry out over the weekend #9' })).toBeVisible()
    await expect(canvas.getByText('Open', { selector: '[data-tone]' })).toBeVisible()
    await expect(canvas.getByText('Assigned ada')).toBeVisible()
    await expect(canvas.getByRole('tab', { name: 'Conversation 1' })).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByRole('link', { name: 'More on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/issues/9')
    await expectGithubLinksOnly(canvasElement)
  },
}

export const Run: Story = {
  args: { pullRequest: undefined },
  render: () => <Canvas><RunDetail target={{ kind: 'run', owner: 'acme', repo: 'garden', run_id: 9001 }} run={runRead} now={fixtureNow} onOpen={onOpen} /></Canvas>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { name: 'CI run 9001' })).toBeVisible()
    await expect(canvas.getByText('Failed', { selector: '[data-tone]' })).toHaveAttribute('data-tone', 'failure')
    await expect(canvas.getByRole('tab', { name: 'Jobs 3' })).toHaveAttribute('aria-selected', 'true')
    await expect(canvas.getByRole('link', { name: 'Logs for build on GitHub' })).toHaveAttribute('href', 'https://github.com/acme/garden/actions/runs/9001/job/1')
    await userEvent.click(canvas.getByRole('button', { name: 'PR #7' }))
    await expect(onOpen).toHaveBeenCalledWith('resource/github/acme/garden/pull-request/7')
    await expectGithubLinksOnly(canvasElement)
  },
}

export const Narrow: Story = {
  args: { pullRequest: pullRequestRead, width: '375px' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const article = canvas.getByRole('article', { name: 'Pull request acme/garden#7' })
    await expect(article.getBoundingClientRect().width).toBeLessThanOrEqual(375)
    await expect(article.scrollWidth).toBeLessThanOrEqual(article.clientWidth)
    await userEvent.click(canvas.getByRole('tab', { name: 'Conversation 6' }))
    await userEvent.keyboard('{End}')
    await expect(canvas.getByRole('tab', { name: 'Files 3' })).toHaveAttribute('aria-selected', 'true')
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg },
  tab: (width: string) => ({ width, maxWidth: '100%', height: '720px', display: 'flex', flexDirection: 'column' }),
})
