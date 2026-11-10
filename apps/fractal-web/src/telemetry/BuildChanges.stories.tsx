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
const builds: Readonly<Record<Build, { readonly identity: BuildChangesIdentity; readonly deploymentId: string | undefined; readonly short: string }>> = {
  current: {
    identity: { rev: revision, dirty: false, commitTs: now - 10_800, buildTs: now - 720, displayVersion: '0.1.0+a7647bb', machineVersion: '0.1.0+nix.a7647bb', sourceKind: 'nix' },
    deploymentId: 'staging',
    short: 'a7647bb · 3h',
  },
  dirty: {
    identity: { rev: revision, dirty: true, commitTs: now - 10_800, buildTs: now - 60, displayVersion: '0.1.0+a7647bb-dirty', machineVersion: '0.1.0+local.a7647bb.dirty', sourceKind: 'local' },
    deploymentId: 'local',
    short: 'a7647bb* · 3h',
  },
  unknown: {
    identity: { dirty: false, displayVersion: '0.1.0', machineVersion: '0.1.0' },
    deploymentId: undefined,
    short: 'n/a · n/a',
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

/** Host-owned seen cursor and load state, mirroring the app's acknowledge-on-open rule without its persistence or chunk loader. */
const BuildChangesStory = ({ scheme, build, changes }: Args) => {
  const dark = scheme === 'dark'
  const fixture = builds[build]
  const listed = changes === 'empty' ? [] : entryIds
  const [cursor, setCursor] = React.useState(() => initialCursor(changes))
  const [baseline, setBaseline] = React.useState(cursor)
  const [load, setLoad] = React.useState(() => initialLoad(changes))
  const [prefetches, setPrefetches] = React.useState(0)
  const hasUnseen = listed.some((id) => !cursor.entryIds.includes(id))
  return (
    <main data-scheme={scheme} data-prefetches={prefetches} {...stylex.props(dark ? darkDevbarTheme : lightDevbarTheme, styles.canvas)}>
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
              onOpen={() => {
                setBaseline(cursor)
                setCursor(acknowledgeEntryIds(listed, cursor))
              }}
              onPrefetch={() => setPrefetches((count) => count + 1)}
              changes={() => <ChangelogView state={load} baseline={baseline} onRetry={() => setLoad({ _tag: 'Ready', manifest })} />}
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
    docs: { description: { component: 'One developer-bar trigger shows the short build identity and the unseen-changes dot; one popover shows build identity and recent changes. Opening the popover shows the changes list and acknowledges it; hover or focus only shows the identity tooltip and warms the changes. The play gate sets the scheme and the 440/1440 width.' } },
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
  'UnknownFailed:contract-fixture',
  'UnknownSeen:contract-fixture',
  'UnknownUnseen:contract-fixture',
]
type Story = StoryObj<typeof meta>

const unseenName = /, unseen changes$/
const dialogName = buildChangesLabel

/** One trigger and one popover; identity inspection never acknowledges; opening does; Escape restores the trigger. Ends open so axe checks the popover. */
const play: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const fixture = builds[args.build]
  const unseen = args.changes !== 'seen' && args.changes !== 'empty'
  const triggers = canvas.getAllByRole('button', { name: /^Build identity: / })
  await expect(triggers).toHaveLength(1)
  const trigger = triggers[0]!
  await expect(trigger).toHaveTextContent(fixture.short)
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  if (unseen) await expect(trigger).toHaveAccessibleName(unseenName)
  else await expect(trigger).not.toHaveAccessibleName(/unseen changes/)
  await expect(trigger.querySelector('[data-devbar-unseen]') !== null).toBe(unseen)

  // Inspecting identity (focus shows the kit tooltip) warms the changes but does not acknowledge them.
  trigger.focus()
  await expect(canvas.getByRole('tooltip')).toBeVisible()
  await expect(Number(canvasElement.querySelector('main')!.dataset.prefetches)).toBeGreaterThan(0)
  if (unseen) await expect(trigger).toHaveAccessibleName(unseenName)

  await userEvent.keyboard('{Enter}')
  const dialog = canvas.getByRole('dialog', { name: dialogName })
  await expect(dialog).toBeVisible()
  await expect(canvas.getAllByRole('dialog')).toHaveLength(1)
  await expect(canvas.queryByRole('tooltip')).toBeNull()
  const panel = within(dialog)
  await expect(panel.getByRole('heading', { name: 'Build' })).toBeVisible()
  await expect(panel.getByRole('heading', { name: "What's new" })).toBeVisible()
  await expect(dialog).toHaveTextContent(fixture.identity.machineVersion)
  if (args.build === 'unknown') await expect(panel.getAllByText('unknown')).toHaveLength(2)
  else await expect(dialog).toHaveTextContent(revision)
  if (args.build === 'dirty') await expect(panel.getByText('uncommitted changes')).toBeVisible()
  else await expect(panel.queryByText('uncommitted changes')).toBeNull()
  if (fixture.deploymentId === undefined) await expect(panel.queryByText('Deployment')).toBeNull()
  else await expect(dialog).toHaveTextContent(`Deployment${fixture.deploymentId}`)
  // The popover shows the list on open, so opening acknowledges it.
  await expect(trigger).not.toHaveAccessibleName(/unseen changes/)
  await expect(trigger.querySelector('[data-devbar-unseen]')).toBeNull()
  await expect(trigger).toHaveAttribute('aria-expanded', 'true')

  if (args.changes === 'unseen') await expect(panel.getByRole('heading', { name: 'New since you last looked' })).toBeVisible()
  else if (args.changes === 'seen') {
    await expect(panel.getByRole('heading', { name: 'Recent' })).toBeVisible()
    await expect(panel.queryByRole('heading', { name: 'New since you last looked' })).toBeNull()
  } else if (args.changes === 'loading') await expect(panel.getByRole('status')).toHaveTextContent('Loading recent changes…')
  else if (args.changes === 'empty') await expect(panel.getByRole('heading', { name: 'No changes yet' })).toBeVisible()
  else {
    await expect(panel.getByRole('heading', { name: "Changes couldn't be loaded" })).toBeVisible()
    await userEvent.click(panel.getByRole('button', { name: 'Retry' }))
    await expect(await panel.findByRole('heading', { name: 'Example build entry' })).toBeVisible()
    await expect(dialog).toHaveFocus()
  }
  await waitFor(() => {
    const bounds = dialog.getBoundingClientRect()
    expect(bounds.left).toBeGreaterThanOrEqual(0)
    expect(bounds.right).toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  })

  await userEvent.keyboard('{Escape}')
  await expect(canvas.queryByRole('dialog', { name: dialogName })).toBeNull()
  await expect(trigger).toHaveFocus()
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')

  // Reopening diffs against the acknowledged cursor: nothing is new any more.
  await userEvent.click(trigger)
  const reopened = canvas.getByRole('dialog', { name: dialogName })
  await expect(reopened).toBeVisible()
  await expect(within(reopened).queryByRole('heading', { name: 'New since you last looked' })).toBeNull()
  if (args.changes === 'unseen' || args.changes === 'failed') await expect(within(reopened).getByRole('heading', { name: 'Recent' })).toBeVisible()
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

const styles = stylex.create({
  canvas: { position: 'relative', boxSizing: 'border-box', width: '100vw', height: '100vh', overflow: 'hidden', backgroundColor: devbarTokens.canvas, color: devbarTokens.text, fontFamily: devbarTokens.fontUi },
  caption: { margin: 0, padding: '1rem', color: devbarTokens.mutedText, fontSize: '0.75rem' },
})
