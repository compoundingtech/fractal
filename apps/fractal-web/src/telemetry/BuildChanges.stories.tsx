/** The developer bar's single build entry: short identity trigger with the unseen dot, and one popover holding build identity and recent changes. */
import { Devbar } from '@overeng/devbar'
import { darkDevbarTheme, lightDevbarTheme } from '@overeng/devbar/themes'
import { devbarTokens } from '@overeng/devbar/tokens.stylex'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'

import { ChangelogView, type ChangelogLoad } from '../changelog/ChangelogView.tsx'
import type { ChangelogManifest } from '../changelog/contract.ts'
import { acknowledgeEntryIds, type ChangelogSeen } from '../changelog/seen.ts'
import { BuildChangesSegment, buildChangesLabel, type BuildChangesIdentity } from './BuildChangesSegment.tsx'

type Build = 'current' | 'dirty' | 'unknown'
type Changes = 'unseen' | 'seen' | 'failed' | 'loading' | 'empty'
type Args = { readonly scheme: 'dark' | 'light'; readonly build: Build; readonly changes: Changes }

/** Fixed Unix seconds: ages render the same on every run. */
const now = 1_800_000_000
const revision = 'a7647bb1234567890abcdef1234567890abcdef1'
const builds: Readonly<Record<Build, { readonly identity: BuildChangesIdentity; readonly deploymentId: string | undefined; readonly short: string; readonly summary: RegExp }>> = {
  current: {
    identity: { rev: revision, dirty: false, commitTs: now - 10_800, buildTs: now - 720, displayVersion: '0.1.0+a7647bb', machineVersion: '0.1.0+nix.a7647bb', sourceKind: 'nix' },
    deploymentId: 'staging',
    short: 'a7647bb · 3h',
    summary: /^Build a7647bb · 3 hours ago$/,
  },
  dirty: {
    identity: { rev: revision, dirty: true, commitTs: now - 10_800, buildTs: now - 60, displayVersion: '0.1.0+a7647bb-dirty', machineVersion: '0.1.0+local.a7647bb.dirty', sourceKind: 'local' },
    deploymentId: 'local',
    short: 'a7647bb* · 3h',
    summary: /^Build a7647bb\* · 3 hours ago · uncommitted changes$/,
  },
  unknown: {
    identity: { dirty: false, displayVersion: '0.1.0', machineVersion: '0.1.0' },
    deploymentId: undefined,
    short: 'n/a · n/a',
    summary: /^Build revision unknown$/,
  },
}
const manifest: ChangelogManifest = {
  schemaVersion: 1, product: 'fractal-web', build: { machineVersion: '0.1.0+nix.a7647bb' },
  entries: [
    { id: '2026-11-10-example-build', date: '2026-11-10', title: 'Example build entry', summary: 'This example entry demonstrates a feature announcement.', kind: 'feature', visibility: 'public', publicPrUrl: 'https://github.com/example/example/pull/3' },
    { id: '2026-11-09-example-search', date: '2026-11-09', title: 'Example search update', summary: 'This example entry demonstrates an improvement announcement.', kind: 'improvement', visibility: 'public' },
    { id: '2026-11-08-example-fix', date: '2026-11-08', title: 'Example alignment fix', summary: 'This example entry demonstrates a fix announcement.', kind: 'fix', visibility: 'private' },
  ],
}
const entryIds = manifest.entries.map((entry) => entry.id)
const initialLoad = (changes: Changes): ChangelogLoad =>
  changes === 'failed' ? { _tag: 'Failed' } : changes === 'loading' ? { _tag: 'Loading' } : { _tag: 'Ready', manifest: changes === 'empty' ? { ...manifest, entries: [] } : manifest }
/** Seen acknowledges every listed ID; otherwise only the oldest was acknowledged by an earlier visit. */
const initialCursor = (changes: Changes): ChangelogSeen => ({ visited: true, entryIds: changes === 'seen' ? entryIds : changes === 'empty' ? [] : [entryIds.at(-1)!] })

/** Like the app's list: diffed against the cursor as it was when the popover mounted it. */
const StoryChanges = ({ cursor, load, onRetry, firstEntryRef }: {
  readonly cursor: ChangelogSeen
  readonly load: ChangelogLoad
  readonly onRetry: () => void
  readonly firstEntryRef: React.RefCallback<HTMLHeadingElement>
}) => {
  const [baseline] = React.useState(cursor)
  return <ChangelogView state={load} baseline={baseline} onRetry={onRetry} firstEntryRef={firstEntryRef} />
}

/** Host-owned seen cursor and load state, mirroring the app's acknowledge-when-seen rule without its persistence or chunk loader. */
const BuildChangesStory = ({ scheme, build, changes }: Args) => {
  const dark = scheme === 'dark'
  const fixture = builds[build]
  const listed = changes === 'empty' ? [] : entryIds
  const [cursor, setCursor] = React.useState(() => initialCursor(changes))
  const [acknowledged, setAcknowledged] = React.useState(0)
  const [load, setLoad] = React.useState(() => initialLoad(changes))
  const [prefetches, setPrefetches] = React.useState(0)
  const hasUnseen = listed.some((id) => !cursor.entryIds.includes(id))
  return (
    <main data-scheme={scheme} data-prefetches={prefetches} data-acknowledged={acknowledged} {...stylex.props(dark ? darkDevbarTheme : lightDevbarTheme, styles.canvas)}>
      <p {...stylex.props(styles.caption)}>Build {build} · changes {changes}</p>
      <Devbar
        placement="container"
        panels={[]}
        openPanel={undefined}
        onOpenPanelChange={() => undefined}
        segments={[{
          id: 'build',
          priority: 1,
          hasAlert: hasUnseen,
          render: () => (
            <BuildChangesSegment
              identity={fixture.identity}
              deploymentId={fixture.deploymentId}
              now={now}
              dark={dark}
              hasUnseen={hasUnseen}
              onChangesSeen={() => {
                setAcknowledged((count) => count + 1)
                setCursor((previous) => acknowledgeEntryIds(listed, previous))
              }}
              onPrefetch={() => setPrefetches((count) => count + 1)}
              changes={(firstEntryRef) => <StoryChanges cursor={cursor} load={load} onRetry={() => setLoad({ _tag: 'Ready', manifest })} firstEntryRef={firstEntryRef} />}
            />
          ),
        }]}
      />
    </main>
  )
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Developer bar build and changes',
  component: BuildChangesStory,
  parameters: {
    scenario: { _tag: 'contract-fixture', reason: 'No catalog slice models build identity or the device-local changelog cursor; fixed identities and local entries exercise current, dirty and unknown builds against unseen, seen, failed, loading and empty changes.' },
    layout: 'fullscreen',
    // Kit defect: the Devbar toggle is named "Toggle developer tools" but reads "Dev tools" (label-content-name-mismatch); https://github.com/overengineeringstudio/effect-utils/issues/2497
    a11y: { context: { include: ['body'], exclude: ['button[aria-label="Toggle developer tools"]'] } },
    docs: { description: { component: 'One developer-bar trigger shows the short build identity and the unseen-changes dot; one popover shows one build line (full identity on demand) above recent changes. Changes are acknowledged, and the dot clears, when the first entry is wholly visible inside the open popover; hover or focus only shows the identity tooltip and warms the changes. The play gate sets the scheme and the width.' } },
  },
  args: { scheme: 'dark', build: 'current', changes: 'unseen' },
  argTypes: {
    scheme: { control: 'radio', options: ['dark', 'light'] },
    build: { control: 'radio', options: ['current', 'dirty', 'unknown'] },
    changes: { control: 'select', options: ['unseen', 'seen', 'failed', 'loading', 'empty'] },
  },
  render: (args) => <BuildChangesStory key={`${args.build}-${args.changes}`} {...args} />,
} satisfies Meta<typeof BuildChangesStory>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'CurrentEmpty:contract-fixture',
  'CurrentFailed:contract-fixture',
  'CurrentLoading:contract-fixture',
  'CurrentSeen:contract-fixture',
  'CurrentUnseen:contract-fixture',
  'DirtyFailed:contract-fixture',
  'DirtySeen:contract-fixture',
  'DirtyUnseen:contract-fixture',
  'NarrowUnseen:contract-fixture',
  'ShortViewportUnseen:contract-fixture',
  'UnknownFailed:contract-fixture',
  'UnknownSeen:contract-fixture',
  'UnknownUnseen:contract-fixture',
]
type Story = StoryObj<typeof meta>

const unseenName = /, unseen changes$/
const dialogName = buildChangesLabel
const frames = (count: number) => new Promise<void>((resolve) => {
  const step = (left: number) => left === 0 ? resolve() : requestAnimationFrame(() => step(left - 1))
  step(count)
})

/** The single trigger, checked before anything is opened. */
const triggerOf = async (canvasElement: HTMLElement, build: Build) => {
  const triggers = within(canvasElement).getAllByRole('button', { name: /^Build identity: / })
  await expect(triggers).toHaveLength(1)
  const trigger = triggers[0]!
  await expect(trigger).toHaveTextContent(builds[build].short)
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  return trigger
}
const acknowledgements = (canvasElement: HTMLElement) => Number(canvasElement.querySelector('main')!.dataset.acknowledged)
const hasDot = (trigger: HTMLElement) => trigger.querySelector('[data-devbar-unseen]') !== null

/**
 * Records the trigger's dot at every DOM change: the dot must clear in the same commit that marks
 * the entries seen, never before and never after.
 */
const watchDot = (canvasElement: HTMLElement, trigger: HTMLElement, unseen: boolean) => {
  const mismatches: string[] = []
  const check = () => {
    const acks = acknowledgements(canvasElement)
    const expected = unseen && acks === 0
    if (hasDot(trigger) !== expected) mismatches.push(`acknowledged=${acks} dot=${hasDot(trigger)}`)
  }
  const observer = new MutationObserver(check)
  observer.observe(canvasElement, { subtree: true, childList: true, attributes: true })
  check()
  return { mismatches, stop: () => observer.disconnect() }
}

/** Wholly inside the dialog's scrollport and the viewport, as the reader sees it on open. */
const expectWhollyVisible = async (element: Element, dialog: HTMLElement) => {
  const box = element.getBoundingClientRect()
  const port = dialog.getBoundingClientRect()
  await expect(box.height).toBeGreaterThan(0)
  await expect(box.top).toBeGreaterThanOrEqual(Math.max(0, port.top))
  await expect(box.bottom).toBeLessThanOrEqual(Math.min(window.innerHeight, port.bottom))
}

/** One trigger and one popover; identity inspection never acknowledges; the visible first entry does; Escape restores the trigger. Ends open so axe checks the popover. */
const play: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const fixture = builds[args.build]
  const unseen = args.changes !== 'seen' && args.changes !== 'empty'
  const trigger = await triggerOf(canvasElement, args.build)
  if (unseen) await expect(trigger).toHaveAccessibleName(unseenName)
  else await expect(trigger).not.toHaveAccessibleName(/unseen changes/)
  await expect(hasDot(trigger)).toBe(unseen)
  const dot = watchDot(canvasElement, trigger, unseen)

  // Inspecting identity (focus shows the kit tooltip) warms the changes but does not acknowledge them.
  // Programmatic focus commits the kit's focus state after the call returns, so the tooltip is awaited.
  trigger.focus()
  await expect(await canvas.findByRole('tooltip')).toBeVisible()
  await expect(Number(canvasElement.querySelector('main')!.dataset.prefetches)).toBeGreaterThan(0)
  await expect(acknowledgements(canvasElement)).toBe(0)

  await userEvent.keyboard('{Enter}')
  const dialog = canvas.getByRole('dialog', { name: dialogName })
  await expect(dialog).toBeVisible()
  await expect(canvas.getAllByRole('dialog')).toHaveLength(1)
  await expect(canvas.queryByRole('tooltip')).toBeNull()
  await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  const panel = within(dialog)
  const summary = dialog.querySelector('summary')!
  await expect(summary.textContent?.replace(/\s+/g, ' ').trim()).toMatch(fixture.summary)
  await expect(panel.getByRole('heading', { name: "What's new" })).toBeVisible()

  if (args.changes === 'loading' || args.changes === 'failed') {
    // Nothing readable yet: the section is in view but no entry is, so nothing is marked seen.
    await frames(3)
    await expect(acknowledgements(canvasElement)).toBe(0)
    await expect(hasDot(trigger)).toBe(true)
    await expect(trigger).toHaveAccessibleName(unseenName)
  }
  if (args.changes === 'loading') await expect(panel.getByRole('status')).toHaveTextContent('Loading recent changes…')
  if (args.changes === 'failed') {
    await expect(panel.getByRole('heading', { name: "Changes couldn't be loaded" })).toBeVisible()
    await userEvent.click(panel.getByRole('button', { name: 'Retry' }))
    await expect(await panel.findByRole('heading', { name: 'Example build entry' })).toBeVisible()
    await expect(dialog).toHaveFocus()
  }
  if (args.changes !== 'loading') {
    // The first readable item is visible without scrolling; seeing it acknowledges and clears the dot together.
    const first = args.changes === 'empty' ? panel.getByRole('heading', { name: 'No changes yet' }) : dialog.querySelector('h3')!
    if (args.changes !== 'empty') await expect(first).toHaveTextContent('Example build entry')
    await expectWhollyVisible(first, dialog)
    await waitFor(() => expect(acknowledgements(canvasElement)).toBe(1))
    await expect(hasDot(trigger)).toBe(false)
    await expect(trigger).not.toHaveAccessibleName(/unseen changes/)
  }
  if (args.changes === 'unseen' || args.changes === 'failed') await expect(panel.getByRole('heading', { name: 'New since you last looked' })).toBeVisible()
  if (args.changes === 'seen') {
    await expect(panel.getByRole('heading', { name: 'Recent' })).toBeVisible()
    await expect(panel.queryByRole('heading', { name: 'New since you last looked' })).toBeNull()
  }

  // Full identity is one disclosure away.
  await userEvent.click(summary)
  await expect(dialog.querySelector('details')).toHaveAttribute('open')
  await expect(dialog).toHaveTextContent(fixture.identity.machineVersion)
  if (args.build === 'unknown') await expect(panel.getAllByText('unknown')).toHaveLength(2)
  else await expect(dialog).toHaveTextContent(revision)
  if (fixture.deploymentId === undefined) await expect(panel.queryByText('Deployment')).toBeNull()
  else await expect(dialog).toHaveTextContent(`Deployment${fixture.deploymentId}`)
  await waitFor(() => {
    const bounds = dialog.getBoundingClientRect()
    expect(bounds.left).toBeGreaterThanOrEqual(0)
    expect(bounds.right).toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  })

  await userEvent.keyboard('{Escape}')
  await expect(canvas.queryByRole('dialog', { name: dialogName })).toBeNull()
  await expect(trigger).toHaveFocus()
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')

  // Reopening diffs against the acknowledged cursor: nothing is new any more, unless nothing was readable.
  await userEvent.click(trigger)
  const reopened = canvas.getByRole('dialog', { name: dialogName })
  await expect(reopened).toBeVisible()
  if (args.changes === 'loading') await expect(hasDot(trigger)).toBe(true)
  else await expect(within(reopened).queryByRole('heading', { name: 'New since you last looked' })).toBeNull()
  if (args.changes === 'unseen' || args.changes === 'failed') await expect(within(reopened).getByRole('heading', { name: 'Recent' })).toBeVisible()
  dot.stop()
  await expect(dot.mismatches).toEqual([])
}

/**
 * The kit bounds the popover to the space beside the bar (`min(24rem, available)`), so on the
 * shortest viewports the first entry can start below the fold. The play applies that bound
 * directly, as the kit computes it for a short viewport, then proves nothing is marked seen until
 * the entry is scrolled into view, and that the dot clears in the same commit.
 */
const shortViewportPlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const trigger = await triggerOf(canvasElement, args.build)
  await expect(hasDot(trigger)).toBe(true)
  const dot = watchDot(canvasElement, trigger, true)
  const popover = canvasElement.querySelector<HTMLElement>(`[role="dialog"][aria-label="${dialogName}"]`)!
  popover.style.maxHeight = '3rem'

  await userEvent.click(trigger)
  const dialog = canvas.getByRole('dialog', { name: dialogName })
  await expect(dialog).toBe(popover)
  await expect(dialog).toBeVisible()
  const first = dialog.querySelector('h3')!
  await expect(first).toHaveTextContent('Example build entry')
  await expect(first.getBoundingClientRect().top).toBeGreaterThanOrEqual(dialog.getBoundingClientRect().bottom)
  await frames(3)
  await expect(acknowledgements(canvasElement)).toBe(0)
  await expect(hasDot(trigger)).toBe(true)
  await expect(trigger).toHaveAccessibleName(unseenName)

  first.scrollIntoView({ block: 'nearest' })
  await waitFor(() => expect(acknowledgements(canvasElement)).toBe(1))
  await expectWhollyVisible(first, dialog)
  await expect(hasDot(trigger)).toBe(false)
  await expect(trigger).not.toHaveAccessibleName(/unseen changes/)
  await expect(within(dialog).getByRole('heading', { name: 'New since you last looked' })).toBeVisible()

  await userEvent.keyboard('{Escape}')
  await expect(canvas.queryByRole('dialog', { name: dialogName })).toBeNull()
  await expect(trigger).toHaveFocus()
  dot.stop()
  await expect(dot.mismatches).toEqual([])
  popover.style.removeProperty('max-height')
  await userEvent.click(trigger)
  await expect(canvas.getByRole('dialog', { name: dialogName })).toBeVisible()
}

export const CurrentUnseen: Story = { play }
export const CurrentSeen: Story = { args: { changes: 'seen' }, play }
export const CurrentFailed: Story = { args: { changes: 'failed' }, play }
export const CurrentLoading: Story = { args: { changes: 'loading' }, play }
export const CurrentEmpty: Story = { args: { changes: 'empty' }, play }
export const DirtyUnseen: Story = { args: { build: 'dirty' }, play }
export const DirtySeen: Story = { args: { build: 'dirty', changes: 'seen' }, play }
export const DirtyFailed: Story = { args: { build: 'dirty', changes: 'failed' }, play }
export const UnknownUnseen: Story = { args: { build: 'unknown' }, play }
export const UnknownSeen: Story = { args: { build: 'unknown', changes: 'seen' }, play }
export const UnknownFailed: Story = { args: { build: 'unknown', changes: 'failed' }, play }
/** Phone width (the gate runs it at 390): the dirty build line wraps and the first entry is still visible on open. */
export const NarrowUnseen: Story = { args: { build: 'dirty' }, play }
/** Phone width with the popover bounded as on the shortest viewport: seen only once the first entry is scrolled into view. */
export const ShortViewportUnseen: Story = { play: shortViewportPlay }

const styles = stylex.create({
  canvas: { position: 'relative', boxSizing: 'border-box', width: '100vw', height: '100vh', overflow: 'hidden', backgroundColor: devbarTokens.canvas, color: devbarTokens.text, fontFamily: devbarTokens.fontUi },
  caption: { margin: 0, padding: '1rem', color: devbarTokens.mutedText, fontSize: '0.75rem' },
})
