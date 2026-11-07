import type { Meta, StoryObj } from '@storybook/react-vite'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as Aria from 'react-aria-components'
import { lightTheme as assistantLightTheme } from './assistant-ui/composition-theme.ts'
import { darkTheme as assistantDarkTheme } from './assistant-ui/embrace-theme.ts'
import { surfaceVars as surface, textVars as ink, spaceVars as space, geometryVars as geometry, geometryNumbers, typeVars as type } from './assistant-ui/composition-tokens.stylex.ts'
import { ChangelogPopover, ChangelogView, type ChangelogLoad } from '../../../apps/fractal-web/src/changelog/ChangelogView.tsx'
import type { ChangelogManifest } from '../../../apps/fractal-web/src/changelog/contract.ts'
import { acknowledgeEntryIds, initialSeen, type ChangelogSeen } from '../../../apps/fractal-web/src/changelog/seen.ts'

const fixtureRevision = '123456789abc'
const fixture: ChangelogManifest = {
  schemaVersion: 1, product: 'fractal-web', build: { machineVersion: `0.1.0+local.${fixtureRevision}` },
  entries: [
    { id: '2026-10-11-drafts', date: '2026-10-11', title: 'Example draft update', summary: 'This example entry demonstrates a feature announcement.', kind: 'feature', visibility: 'public', publicPrUrl: 'https://github.com/example/example/pull/1' },
    { id: '2026-10-10-reconnect', date: '2026-10-10', title: 'Example connection update', summary: 'This example entry demonstrates an improvement announcement.', kind: 'improvement', visibility: 'public', publicPrUrl: 'https://github.com/example/example/pull/2' },
  ],
}
type Scenario = 'unseen' | 'seen' | 'first' | 'loading' | 'empty' | 'failure'
const scenario = (value: Scenario): { readonly state: ChangelogLoad; readonly baseline: ChangelogSeen } => ({
  state: value === 'loading' ? { _tag: 'Loading' } : value === 'failure' ? { _tag: 'Failed' } : { _tag: 'Ready', manifest: value === 'empty' ? { ...fixture, entries: [] } : fixture },
  baseline: value === 'first' ? initialSeen : { visited: true, entryIds: value === 'seen' ? fixture.entries.map((entry) => entry.id) : ['2026-10-10-reconnect'] },
})
type Scheme = 'dark' | 'light'
/** Fills the window, so the Viewport toolbar (and the gate's 440 and 1440 windows) set the width. */
const ChangelogStoryHarness = ({ mode = 'unseen', scheme = 'dark', initiallyOpen = true }: { readonly mode?: Scenario; readonly scheme?: Scheme; readonly initiallyOpen?: boolean }) => {
  const dark = scheme === 'dark'
  const current = scenario(mode)
  const [open, setOpen] = React.useState(initiallyOpen)
  const [cursor, setCursor] = React.useState(() => initiallyOpen && current.state._tag === 'Ready' ? acknowledgeEntryIds(current.state.manifest.entries.map((entry) => entry.id), current.baseline) : current.baseline)
  const [baseline, setBaseline] = React.useState(current.baseline)
  const [state, setState] = React.useState(current.state)
  return <main data-changelog-story="" data-scheme={dark ? 'dark' : 'light'} {...stylex.props(styles.canvas, styles.fill, dark ? assistantDarkTheme : assistantLightTheme)}>
    <p {...stylex.props(styles.caption)}>Fractal · {mode}</p>
    <footer data-changelog-anchor="" {...stylex.props(styles.bar)}><span>{fixture.build.machineVersion}</span><ChangelogPopover dark={dark} isOpen={open} onOpenChange={(next) => {
      if (next && state._tag === 'Ready') { setBaseline(cursor); setCursor(acknowledgeEntryIds(state.manifest.entries.map((entry) => entry.id), cursor)) }
      setOpen(next)
    }} hasUnseen={cursor.visited && state._tag === 'Ready' && state.manifest.entries.some((entry) => !cursor.entryIds.includes(entry.id))}>
      <ChangelogView state={state} baseline={baseline} onRetry={() => setState({ _tag: 'Ready', manifest: fixture })} buildRevision={fixtureRevision} />
    </ChangelogPopover></footer>
  </main>
}
/** Matches the live zero-height theme holder around its fixed visible status bar. */
const ChangelogDesktopAnchorStoryHarness = ({ scheme = 'dark' }: { readonly scheme?: Scheme }) => {
  const dark = scheme === 'dark'
  const [open, setOpen] = React.useState(false)
  return <main data-changelog-desktop-story="" {...stylex.props(styles.canvas, styles.fullCanvas, dark ? assistantDarkTheme : assistantLightTheme)}>
    <div data-testid="wf-devbar">
      <aside {...stylex.props(styles.desktopBar)}>
        <ChangelogPopover dark={dark} isOpen={open} onOpenChange={setOpen} hasUnseen>
          <ChangelogView {...scenario('unseen')} onRetry={() => undefined} buildRevision={fixtureRevision} />
        </ChangelogPopover>
      </aside>
    </div>
    <p {...stylex.props(styles.caption)}>A zero-height theme holder surrounds the fixed status bar.</p>
  </main>
}
const desktopAnchorPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const document = canvasElement.ownerDocument
  const body = within(document.body)
  const holder = canvasElement.querySelector<HTMLElement>('[data-testid="wf-devbar"]')!
  const bar = holder.querySelector<HTMLElement>('aside')!
  expect(holder.getBoundingClientRect().height).toBe(0)
  expect(holder.getBoundingClientRect().top).toBe(0)
  expect(document.defaultView!.getComputedStyle(bar).position).toBe('fixed')
  const trigger = body.getByRole('button', { name: 'Changes, unread' })
  await userEvent.click(trigger)
  const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(dialog).toBeVisible()
  expect(dialog.querySelector('[title]')).toBeNull()
  const popover = document.querySelector<HTMLElement>('[data-changelog-popover]')!
  await waitFor(() => expect(Math.abs(bar.getBoundingClientRect().top - popover.getBoundingClientRect().bottom - geometryNumbers.contextOffset)).toBeLessThan(0.75))
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(body.queryByRole('dialog', { name: 'Recent changes' })).toBeNull())
  await waitFor(() => expect(trigger).toHaveFocus())
  canvasElement.dataset.changelogDesktopAnchorProof = 'passed'
}
/** Native overflow lifetime matches the kit: closing +N unmounts its disclosure trigger. */
const ChangelogOverflowStoryHarness = ({ scheme = 'dark' }: { readonly scheme?: Scheme }) => {
  const dark = scheme === 'dark'
  const id = React.useId()
  const overflow = React.useRef<HTMLDivElement>(null)
  const [expanded, setExpanded] = React.useState(false)
  const [open, setOpen] = React.useState(false)
  return <main data-changelog-overflow-story="" {...stylex.props(styles.canvas, styles.fill, dark ? assistantDarkTheme : assistantLightTheme)}>
    <div data-testid="light-dismiss-target" {...stylex.props(styles.dismissTarget)}>Click this workspace area to dismiss both popovers.</div>
    <footer {...stylex.props(styles.bar)}>
      <Aria.Button aria-label="+N more status items" aria-haspopup="dialog" aria-controls={id} aria-expanded={expanded} onPress={() => {
        setExpanded(true)
        overflow.current?.showPopover()
        overflow.current?.focus()
      }}>+N</Aria.Button>
    </footer>
    <div ref={overflow} id={id} popover="auto" role="dialog" aria-label="More status items" tabIndex={-1}
      onToggle={(event) => { const next = event.newState === 'open'; setExpanded(next); if (!next) setOpen(false) }} {...stylex.props(styles.overflow, dark ? assistantDarkTheme : assistantLightTheme)}>
      {expanded ? <ChangelogPopover dark={dark} isOpen={open} onOpenChange={setOpen} hasUnseen>
        <ChangelogView {...scenario('unseen')} onRetry={() => undefined} buildRevision={fixtureRevision} />
      </ChangelogPopover> : null}
    </div>
  </main>
}
const overflowLightDismissPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const document = canvasElement.ownerDocument
  const body = within(document.body)
  const trigger = body.getByRole('button', { name: '+N more status items' })
  const controls = trigger.getAttribute('aria-controls')!
  const holder = document.getElementById(controls)!
  const open = async () => {
    await userEvent.click(trigger)
    await userEvent.click(await body.findByRole('button', { name: 'Changes, unread' }))
    const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
    expect(dialog).toBeVisible()
    expect(dialog.querySelector('[title]')).toBeNull()
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true))
  }
  const dismiss = async () => {
    // Synthetic userEvent cannot produce trusted native light-dismiss; close the parent directly.
    holder.hidePopover()
    await waitFor(() => expect(body.queryByRole('dialog', { name: 'Recent changes' })).toBeNull())
    await waitFor(() => expect(body.queryByRole('dialog', { name: 'More status items' })).toBeNull())
  }
  // Negative control disconnects the only surviving focus target before the child mounts.
  trigger.setAttribute('aria-controls', 'disconnected-fallback-control')
  await open()
  await dismiss()
  await waitFor(() => expect(document.activeElement).toBe(document.body))
  trigger.setAttribute('aria-controls', controls)
  await open()
  await dismiss()
  await waitFor(() => expect(trigger).toHaveFocus())
  canvasElement.dataset.changelogFocusProof = 'passed'
}
const meta = {
  excludeStories: ['scenarioExemptions'],
  // Pinned so story ids, and the gate policies that name them, survive the retitle.
  id: 'fractal-app-changelog',
  title: 'Surfaces/Changelog', component: ChangelogStoryHarness,
  render: (args) => <ChangelogStoryHarness key={args.mode} {...args} />,
  parameters: { scenario: { _tag: 'contract-fixture', reason: "The catalog has no changelog slice; local release entries exercise seen, loading and failure contracts." },
    layout: 'fullscreen',
    docs: { description: { component: 'Synthetic changelog entries demonstrate loading, retry, acknowledgement, desktop anchoring and native overflow focus restoration. Links and build identity are examples, not release provenance.' } },
  },
  args: { mode: 'unseen', scheme: 'dark' },
  argTypes: { mode: { control: 'select', options: ['unseen', 'seen', 'first', 'loading', 'empty', 'failure'] }, scheme: { control: 'inline-radio', options: ['dark', 'light'] } },
} satisfies Meta<typeof ChangelogStoryHarness>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Acknowledgement1440Dark:contract-fixture',
  'AllStates:contract-fixture',
  'DesktopAnchor1440Dark:contract-fixture',
  'Empty1440Dark:contract-fixture',
  'FirstVisit1440Dark:contract-fixture',
  'LoadFailure1440Dark:contract-fixture',
  'Loading1440Dark:contract-fixture',
  'OverflowLightDismiss1440Dark:contract-fixture',
  'Seen1440Dark:contract-fixture',
  'Unseen1440Dark:contract-fixture',
]
type Story = StoryObj<typeof meta>
const visibleStatePlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const body = within(canvasElement.ownerDocument.body)
  const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(dialog).toBeVisible()
  const content = within(dialog)
  if (args.mode === 'failure') {
    expect(content.getByRole('heading', { name: "Changes couldn't be loaded" })).toBeVisible()
    await userEvent.click(content.getByRole('button', { name: 'Retry' }))
    expect(await content.findByRole('heading', { name: 'Example draft update' })).toBeVisible()
  } else if (args.mode === 'loading') {
    expect(content.getByRole('status')).toHaveTextContent('Loading recent changes…')
  } else if (args.mode === 'empty') {
    expect(content.getByRole('heading', { name: 'No changes yet' })).toBeVisible()
  } else {
    expect(content.getByRole('heading', { name: 'Example draft update' })).toBeVisible()
    if (args.mode === 'unseen') expect(content.getByRole('heading', { name: 'New since you last looked' })).toBeVisible()
    else expect(content.queryByRole('heading', { name: 'New since you last looked' })).toBeNull()
  }
  expect(dialog.querySelector('[title]')).toBeNull()
  await waitFor(() => {
    const bounds = dialog.getBoundingClientRect()
    expect(bounds.left).toBeGreaterThanOrEqual(0)
    expect(bounds.right).toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  })
}
const acknowledgementPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const body = within(canvasElement.ownerDocument.body)
  const trigger = body.getByRole('button', { name: 'Changes, unread' })
  await userEvent.click(trigger)
  const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(within(dialog).getByRole('heading', { name: 'New since you last looked' })).toBeVisible()
  await userEvent.click(body.getByRole('button', { name: 'Close recent changes' }))
  await waitFor(() => expect(body.queryByRole('dialog', { name: 'Recent changes' })).toBeNull())
  await waitFor(() => expect(trigger).toHaveFocus())
  await userEvent.click(trigger)
  const reopened = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(body.queryByTestId('changelog-unseen')).toBeNull()
  expect(within(reopened).queryByRole('heading', { name: 'New since you last looked' })).toBeNull()
  expect(within(reopened).getByRole('heading', { name: 'Recent' })).toBeVisible()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(body.queryByRole('dialog', { name: 'Recent changes' })).toBeNull())
  await waitFor(() => expect(trigger).toHaveFocus())
}
export const Unseen1440Dark: Story = { name: 'Unseen', play: visibleStatePlay }
export const Seen1440Dark: Story = { name: 'Seen', args: { mode: 'seen' }, play: visibleStatePlay }
export const Empty1440Dark: Story = { name: 'Empty', args: { mode: 'empty' }, play: visibleStatePlay }
export const LoadFailure1440Dark: Story = { name: 'Load failure', args: { mode: 'failure' }, play: visibleStatePlay }
export const Loading1440Dark: Story = { name: 'Loading', args: { mode: 'loading' }, play: visibleStatePlay }
export const FirstVisit1440Dark: Story = { name: 'First visit', args: { mode: 'first' }, play: visibleStatePlay }
export const Acknowledgement1440Dark: Story = { name: 'Acknowledgement', args: { initiallyOpen: false }, play: acknowledgementPlay }
export const OverflowLightDismiss1440Dark: Story = { name: 'Overflow light dismiss', render: (args) => <ChangelogOverflowStoryHarness {...args} />, play: overflowLightDismissPlay }
export const DesktopAnchor1440Dark: Story = { name: 'Desktop anchor', render: (args) => <ChangelogDesktopAnchorStoryHarness {...args} />, play: desktopAnchorPlay }
const ChangelogStateCard = ({ mode, dark }: { readonly mode: Scenario; readonly dark: boolean }) => {
  const current = scenario(mode)
  const [state, setState] = React.useState(current.state)
  return <article {...stylex.props(styles.card, dark ? assistantDarkTheme : assistantLightTheme)}><h1>{dark ? 'Dark' : 'Light'} · {mode}</h1><ChangelogView state={state} baseline={current.baseline} onRetry={() => setState({ _tag: 'Ready', manifest: fixture })} buildRevision={fixtureRevision} /></article>
}
export const AllStates: Story = {
  render: () => <main {...stylex.props(styles.matrix)}>{([true, false] as const).flatMap((dark) => (['unseen', 'seen', 'first', 'loading', 'empty', 'failure'] as const).map((mode) => <ChangelogStateCard key={`${dark}-${mode}`} mode={mode} dark={dark} />))}</main>,
  play: async ({ canvasElement }) => {
    expect(canvasElement.querySelectorAll('article')).toHaveLength(12)
    expect(canvasElement.querySelector('[title]')).toBeNull()
    const canvas = within(canvasElement)
    expect(canvas.getAllByRole('status')).toHaveLength(2)
    expect(canvas.getAllByRole('button', { name: 'Retry' })).toHaveLength(2)
    expect(canvas.getAllByRole('heading', { name: 'No changes yet' })).toHaveLength(2)
  },
}
const styles = stylex.create({
  canvas: { position: 'relative', maxWidth: '100vw', minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg, fontFamily: type.fontSans },
  fill: { width: '100%' },
  fullCanvas: { width: '100vw', height: '100vh' },
  caption: { padding: space.section, margin: 0, color: ink.fgMuted, fontSize: type.codeSize },
  bar: { position: 'absolute', bottom: 0, left: 0, right: 0, display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: space.md, padding: space.lg, backgroundColor: surface.raised, fontSize: type.metaSize },
  matrix: { display: 'grid', gridTemplateColumns: `repeat(auto-fit, minmax(${geometry.tooltipMax}, 1fr))`, gap: space.xl, padding: space.xl, backgroundColor: surface.canvas, color: ink.fg },
  card: { backgroundColor: surface.raised, color: ink.fg, padding: space.xl, fontFamily: type.fontSans },
  desktopBar: { position: 'fixed', bottom: 0, left: 0, right: 0, height: geometry.controlLg, boxSizing: 'border-box', display: 'flex', justifyContent: 'flex-end', alignItems: 'center', paddingBlock: 0, paddingInline: space.lg, backgroundColor: surface.raised, fontSize: type.metaSize },
  dismissTarget: { padding: space.section, minHeight: geometry.threadViewportMin, color: ink.fgMuted },
  overflow: { position: 'fixed', inset: 'auto', bottom: `calc(${geometry.controlLg} + ${space.control})`, right: space.lg, margin: 0, padding: space.lg, backgroundColor: surface.raised, color: ink.fg },
})
