/** The GitHub hover card over the normative plugin fixtures: every read state, and a Markdown link that opens it on hover or focus. Synthetic `acme/garden` data only. */
import { markdownBlock } from '@fractal/client/markdown'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Markdown, MarkdownLinkPreviewContext } from '@fractal/ui/assistant-ui'
import { assistantDarkTheme } from '@fractal/ui/assistant-ui/shell'
import { surfaceVars as surface, textVars as ink } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../../../ui-compat/live-theme.stylex.ts'
import { GithubReaderProvider, GithubReadUnsupported, type GithubReader, type Loaded } from '../data.ts'
import { checksRead, commentRead, conversationRead, filesRead, fixtureNow, goneRead, issueRead, movedRead, notFoundRead, pullRequestRead, runRead, stalePullRequestRead, unavailableRead } from '../fixtures.ts'
import { parseGithubUrl } from '../refs.ts'
import type { GithubRef } from '../refs.ts'
import { GithubCard, GithubHoverLink } from './GithubCard.tsx'

const pr: GithubRef = { kind: 'pull-request', owner: 'acme', repo: 'garden', number: 7 }

/** Answers every read from the fixtures; refs it does not know read as not found. */
export const fixtureReader: GithubReader = {
  item: async ref => ref.kind === 'pull-request' ? (ref.number === 7 ? pullRequestRead : notFoundRead)
    // GitHub's issues read answers for PR #7 too, marked as a PR.
    : ref.kind === 'issue' ? (ref.number === 7 && issueRead.status === 'found' ? { ...issueRead, item: { ...issueRead.item, number: 7, is_pull_request: true } } : issueRead)
    : ref.kind === 'comment' ? commentRead : runRead,
  conversation: async () => conversationRead,
  checks: async () => checksRead,
  files: async () => filesRead,
}

const Canvas = ({ children }: { readonly children: React.ReactNode }) =>
  <main data-scheme="dark" {...stylex.props(styles.canvas, liveLegacyTheme, assistantDarkTheme)}>{children}</main>

interface CardProps { readonly target: GithubRef; readonly loaded: Loaded<never> | Loaded<unknown> }
const Card = ({ target, loaded }: CardProps) => <Canvas><div {...stylex.props(styles.frame)}><GithubCard target={target} loaded={loaded as never} now={fixtureNow} /></div></Canvas>

const meta = {
  excludeStories: ['scenarioExemptions', 'fixtureReader'],
  title: 'wf/Plugins/GitHub/Hover card',
  component: Card,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The normative GitHub plugin wire fixtures pin the card for every kind and read state.' } },
} satisfies Meta<typeof Card>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Comment:contract-fixture', 'Error:contract-fixture', 'Gone:contract-fixture', 'Issue:contract-fixture', 'Loading:contract-fixture', 'MarkdownLinks:contract-fixture', 'Moved:contract-fixture', 'NotFound:contract-fixture', 'OlderDaemon:contract-fixture', 'PullRequest:contract-fixture', 'Run:contract-fixture', 'Stale:contract-fixture', 'Unavailable:contract-fixture']

const noImages = async (root: HTMLElement) => expect(root.querySelectorAll('img')).toHaveLength(0)

export const PullRequest: Story = {
  args: { target: pr, loaded: { _tag: 'Read', read: pullRequestRead } },
  play: async ({ canvasElement }) => {
    const card = within(canvasElement)
    await expect(card.getByText('acme/garden#7')).toBeVisible()
    await expect(card.getByText('Water the beds on a schedule')).toBeVisible()
    await expect(card.getByText('Open')).toBeVisible()
    await expect(card.getByText('ada')).toBeVisible()
    await expect(card.getByText(/Changes requested/)).toBeVisible()
    // The body's image stays words; nothing is fetched from GitHub's asset host.
    await expect(card.getByText(/\[image: moisture chart\]/)).toBeVisible()
    await noImages(canvasElement)
  },
}
export const Stale: Story = {
  args: { target: pr, loaded: { _tag: 'Read', read: stalePullRequestRead } },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText('Merged')).toBeVisible()
    await expect(within(canvasElement).getByText(/^stale · fetched/)).toBeVisible()
  },
}
export const Issue: Story = {
  args: { target: { kind: 'issue', owner: 'acme', repo: 'garden', number: 9 }, loaded: { _tag: 'Read', read: issueRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText(/Assigned ada/)).toBeVisible() },
}
export const Comment: Story = {
  args: { target: { kind: 'comment', owner: 'acme', repo: 'garden', comment_id: 1832451234, number: 7 }, loaded: { _tag: 'Read', read: commentRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText(/3 reactions/)).toBeVisible() },
}
export const Run: Story = {
  args: { target: { kind: 'run', owner: 'acme', repo: 'garden', run_id: 9001 }, loaded: { _tag: 'Read', read: runRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('Failed')).toBeVisible() },
}
export const Loading: Story = {
  args: { target: pr, loaded: { _tag: 'Loading' } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('Loading from GitHub…')).toBeVisible() },
}
export const Error: Story = {
  args: { target: pr, loaded: { _tag: 'Failed', message: 'Fractal refused the GitHub read (403)' } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('Preview unavailable')).toBeVisible() },
}
export const Moved: Story = {
  args: { target: pr, loaded: { _tag: 'Read', read: movedRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText(/Now at acme-garden\/garden#7/)).toBeVisible() },
}
export const Gone: Story = {
  args: { target: pr, loaded: { _tag: 'Read', read: goneRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('Deleted')).toBeVisible() },
}
export const NotFound: Story = {
  args: { target: pr, loaded: { _tag: 'Read', read: notFoundRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText('Not found or no access')).toBeVisible() },
}
export const Unavailable: Story = {
  args: { target: pr, loaded: { _tag: 'Read', read: unavailableRead } },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).getByText(/Retry in 2m/)).toBeVisible() },
}

const preview = (href: string, link: React.ReactElement) => {
  const target = parseGithubUrl(href)
  return target === undefined ? undefined : <GithubHoverLink target={target} now={fixtureNow}>{link}</GithubHoverLink>
}
const markdown = 'Fixes [the PR](https://github.com/acme/garden/pull/7), see [issue 9](https://github.com/acme/garden/issues/9) and [docs](https://example.com/docs "Docs title").'

/** Links in Markdown: GitHub ones get the card on hover or keyboard focus and lose the native title; others are untouched. */
export const MarkdownLinks: Story = {
  args: { target: pr, loaded: { _tag: 'Loading' } },
  render: () => <Canvas><GithubReaderProvider value={fixtureReader}><MarkdownLinkPreviewContext.Provider value={preview}><div {...stylex.props(styles.prose)}><Markdown text={markdownBlock(markdown)} /></div></MarkdownLinkPreviewContext.Provider></GithubReaderProvider></Canvas>,
  play: async ({ canvasElement }) => {
    const page = within(canvasElement)
    const link = page.getByRole('link', { name: 'the PR' })
    await expect(link).not.toHaveAttribute('title')
    await expect(page.getByRole('link', { name: 'docs' })).toHaveAttribute('title', 'Docs title')
    await userEvent.tab()
    await expect(link).toHaveFocus()
    const tooltip = await within(document.body).findByRole('tooltip', {}, { timeout: 2000 })
    await waitFor(() => expect(within(tooltip).getByText('Water the beds on a schedule')).toBeVisible())
    await noImages(tooltip)
  },
}

const unsupported = async (): Promise<never> => { throw new GithubReadUnsupported('no route') }
const olderDaemon: GithubReader = { item: unsupported, conversation: unsupported, checks: unsupported, files: unsupported }

/** A daemon from before the plugin reads: focusing a GitHub link shows no card and no error, just the link. */
export const OlderDaemon: Story = {
  args: { target: pr, loaded: { _tag: 'Loading' } },
  render: () => <Canvas><GithubReaderProvider value={olderDaemon}><MarkdownLinkPreviewContext.Provider value={preview}><div {...stylex.props(styles.prose)}><Markdown text={markdownBlock(markdown)} /></div></MarkdownLinkPreviewContext.Provider></GithubReaderProvider></Canvas>,
  play: async ({ canvasElement }) => {
    const link = within(canvasElement).getByRole('link', { name: 'the PR' })
    await userEvent.tab()
    await expect(link).toHaveFocus()
    await new Promise(resolve => setTimeout(resolve, 400))
    await expect(within(document.body).queryByRole('tooltip')).toBeNull()
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: '24px', backgroundColor: surface.canvas, color: ink.fg },
  // The kit card frame's surface and gutter; the card itself is the kit's 340px.
  frame: { width: 'max-content', maxWidth: '100%', padding: '12px', backgroundColor: surface.raised, borderRadius: '8px' },
  prose: { maxWidth: '40rem' },
})
