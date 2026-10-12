import type { Meta, StoryObj } from '@storybook/react-vite'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as Aria from 'react-aria-components'
import { lightTheme as assistantLightTheme } from './assistant-ui/composition-theme.ts'
import { darkTheme as assistantDarkTheme } from './assistant-ui/embrace-theme.ts'
import { surfaceVars as surface, textVars as ink } from './assistant-ui/composition-tokens.stylex.ts'
import { ChangelogPopover, ChangelogView, type ChangelogLoad } from '../../../apps/fractal-web/src/changelog/ChangelogView.tsx'
import type { ChangelogManifest } from '../../../apps/fractal-web/src/changelog/contract.ts'
import { acknowledgeEntryIds, initialSeen, type ChangelogSeen } from '../../../apps/fractal-web/src/changelog/seen.ts'

const fixtureRevision = '123456789abc'
const fixture: ChangelogManifest = {
  schemaVersion: 1, product: 'fractal-web', build: { machineVersion: `0.1.0+local.${fixtureRevision}` },
  entries: [
    { id: '2026-10-11-drafts', date: '2026-10-11', title: 'Drafts stay with your browser', summary: 'Browser-local drafts persist while you move between conversations.', kind: 'feature', visibility: 'public', publicPrUrl: 'https://github.com/compoundingtech/smalltalk/pull/2349' },
    { id: '2026-10-10-reconnect', date: '2026-10-10', title: 'Clearer connection status', summary: 'Explicit reconnect and status wording makes connection changes easier to follow.', kind: 'improvement', visibility: 'public', publicPrUrl: 'https://github.com/compoundingtech/smalltalk/pull/2349' },
  ],
}
type Scenario = 'unseen' | 'seen' | 'first' | 'loading' | 'empty' | 'failure'
const scenario = (value: Scenario): { readonly state: ChangelogLoad; readonly baseline: ChangelogSeen } => ({
  state: value === 'loading' ? { _tag: 'Loading' } : value === 'failure' ? { _tag: 'Failed' } : { _tag: 'Ready', manifest: value === 'empty' ? { ...fixture, entries: [] } : fixture },
  baseline: value === 'first' ? initialSeen : { visited: true, entryIds: value === 'seen' ? fixture.entries.map((entry) => entry.id) : ['2026-10-10-reconnect'] },
})
const ChangelogStoryHarness = ({ mode = 'unseen', dark = true, width = 1440 }: { readonly mode?: Scenario; readonly dark?: boolean; readonly width?: number }) => {
  const current = scenario(mode)
  const [open, setOpen] = React.useState(true)
  const [cursor, setCursor] = React.useState(current.baseline)
  const [baseline, setBaseline] = React.useState(current.baseline)
  const [state, setState] = React.useState(current.state)
  return <main data-changelog-story="" data-scheme={dark ? 'dark' : 'light'} style={{ width, maxWidth: '100vw', height: 900 }} {...stylex.props(styles.canvas, dark ? assistantDarkTheme : assistantLightTheme)}>
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
const ChangelogDesktopAnchorStoryHarness = ({ dark = true }: { readonly dark?: boolean }) => {
  const [open, setOpen] = React.useState(false)
  return <main data-changelog-desktop-story="" style={{ width: '100vw', height: '100vh' }} {...stylex.props(styles.canvas, dark ? assistantDarkTheme : assistantLightTheme)}>
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
  const trigger = body.getByRole('button', { name: 'Recent changes' })
  await userEvent.click(trigger)
  const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(dialog).toBeVisible()
  expect(dialog.querySelector('[title]')).toBeNull()
  const popover = document.querySelector<HTMLElement>('[data-changelog-popover]')!
  await waitFor(() => expect(Math.abs(bar.getBoundingClientRect().top - popover.getBoundingClientRect().bottom - 8)).toBeLessThan(0.75))
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(body.queryByRole('dialog', { name: 'Recent changes' })).toBeNull())
  await waitFor(() => expect(trigger).toHaveFocus())
  canvasElement.dataset.changelogDesktopAnchorProof = 'passed'
}
/** Native overflow lifetime matches the kit: closing +N unmounts its disclosure trigger. */
const ChangelogOverflowStoryHarness = ({ dark = true, width = 440 }: { readonly dark?: boolean; readonly width?: number }) => {
  const id = React.useId()
  const overflow = React.useRef<HTMLDivElement>(null)
  const [expanded, setExpanded] = React.useState(false)
  const [open, setOpen] = React.useState(false)
  return <main data-changelog-overflow-story="" style={{ width, maxWidth: '100vw', height: 900 }} {...stylex.props(styles.canvas, dark ? assistantDarkTheme : assistantLightTheme)}>
    <div data-testid="light-dismiss-target" {...stylex.props(styles.dismissTarget)}>Click this workspace area to dismiss both popovers.</div>
    <footer {...stylex.props(styles.bar)}>
      <Aria.Button aria-label="More status items" aria-haspopup="dialog" aria-controls={id} aria-expanded={expanded} onPress={() => {
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
  const trigger = body.getByRole('button', { name: 'More status items' })
  const controls = trigger.getAttribute('aria-controls')!
  const holder = document.getElementById(controls)!
  const open = async () => {
    await userEvent.click(trigger)
    await userEvent.click(await body.findByRole('button', { name: 'Recent changes' }))
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
  title: 'Fractal/App/Changelog', component: ChangelogStoryHarness,
  render: (args) => <ChangelogStoryHarness key={args.mode} {...args} />,
  parameters: {
    layout: 'fullscreen',
    viewport: { options: {
      changelog1440: { name: 'Changelog desktop', styles: { width: '1440px', height: '900px' }, type: 'desktop' },
      changelog440: { name: 'Changelog narrow', styles: { width: '440px', height: '900px' }, type: 'mobile' },
    } },
    docs: { description: { component: 'Live story and component contract ported from 0c24b983b. Deterministic public fixtures cover merged main PR #2349 only; private preview content and open PRs #2470, #2471, #2473, #2474 and #2476 are excluded. Both themes, 440/1440 widths, loading, retry, acknowledgement, desktop anchoring and native overflow focus restoration are covered.' } },
  },
  globals: { viewport: { value: 'changelog1440', isRotated: false } },
  args: { mode: 'unseen', dark: true, width: 1440 },
  argTypes: { mode: { control: 'select', options: ['unseen', 'seen', 'first', 'loading', 'empty', 'failure'] }, dark: { control: 'boolean' }, width: { control: 'select', options: [1440, 440] } },
} satisfies Meta<typeof ChangelogStoryHarness>
export default meta
type Story = StoryObj<typeof meta>
const visibleStatePlay: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const body = within(canvasElement.ownerDocument.body)
  const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(dialog).toBeVisible()
  const content = within(dialog)
  if (args.mode === 'failure') {
    expect(content.getByRole('heading', { name: "Changes couldn't be loaded" })).toBeVisible()
    await userEvent.click(content.getByRole('button', { name: 'Retry' }))
    expect(await content.findByRole('heading', { name: 'Drafts stay with your browser' })).toBeVisible()
  } else if (args.mode === 'loading') {
    expect(content.getByRole('status')).toHaveTextContent('Loading recent changes…')
  } else if (args.mode === 'empty') {
    expect(content.getByRole('heading', { name: 'No changes yet' })).toBeVisible()
  } else {
    expect(content.getByRole('heading', { name: 'Drafts stay with your browser' })).toBeVisible()
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
  const dialog = await body.findByRole('dialog', { name: 'Recent changes' })
  expect(within(dialog).getByRole('heading', { name: 'New since you last looked' })).toBeVisible()
  await userEvent.click(body.getByRole('button', { name: 'Close recent changes' }))
  await waitFor(() => expect(body.queryByRole('dialog', { name: 'Recent changes' })).toBeNull())
  const trigger = body.getByRole('button', { name: 'Recent changes' })
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
export const Unseen1440Dark: Story = { play: visibleStatePlay }
export const Unseen1440Light: Story = { args: { dark: false }, play: visibleStatePlay }
export const Unseen440Dark: Story = { args: { width: 440 }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const Unseen440Light: Story = { args: { width: 440, dark: false }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const Seen1440Dark: Story = { args: { mode: 'seen' }, play: visibleStatePlay }
export const Seen1440Light: Story = { args: { mode: 'seen', dark: false }, play: visibleStatePlay }
export const Seen440Dark: Story = { args: { mode: 'seen', width: 440 }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const Seen440Light: Story = { args: { mode: 'seen', width: 440, dark: false }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const Empty1440Dark: Story = { args: { mode: 'empty' }, play: visibleStatePlay }
export const Empty1440Light: Story = { args: { mode: 'empty', dark: false }, play: visibleStatePlay }
export const Empty440Dark: Story = { args: { mode: 'empty', width: 440 }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const Empty440Light: Story = { args: { mode: 'empty', width: 440, dark: false }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const LoadFailure1440Dark: Story = { args: { mode: 'failure' }, play: visibleStatePlay }
export const LoadFailure1440Light: Story = { args: { mode: 'failure', dark: false }, play: visibleStatePlay }
export const LoadFailure440Dark: Story = { args: { mode: 'failure', width: 440 }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const LoadFailure440Light: Story = { args: { mode: 'failure', width: 440, dark: false }, globals: { viewport: { value: 'changelog440', isRotated: false } }, play: visibleStatePlay }
export const Loading: Story = { args: { mode: 'loading' }, play: visibleStatePlay }
export const FirstVisit: Story = { args: { mode: 'first' }, play: visibleStatePlay }
export const Acknowledgement: Story = { play: acknowledgementPlay }
export const OverflowLightDismiss440Dark: Story = { args: { width: 440 }, globals: { viewport: { value: 'changelog440', isRotated: false } }, render: (args) => <ChangelogOverflowStoryHarness {...args} />, play: overflowLightDismissPlay }
export const OverflowLightDismiss440Light: Story = { args: { width: 440, dark: false }, globals: { viewport: { value: 'changelog440', isRotated: false } }, render: (args) => <ChangelogOverflowStoryHarness {...args} />, play: overflowLightDismissPlay }
export const DesktopAnchorDark: Story = { render: () => <ChangelogDesktopAnchorStoryHarness />, play: desktopAnchorPlay }
export const DesktopAnchorLight: Story = { render: () => <ChangelogDesktopAnchorStoryHarness dark={false} />, play: desktopAnchorPlay }
const ChangelogStateCard = ({ mode, dark }: { readonly mode: Scenario; readonly dark: boolean }) => {
  const current = scenario(mode)
  const [state, setState] = React.useState(current.state)
  return <article {...stylex.props(styles.card, dark ? assistantDarkTheme : assistantLightTheme)}><h1>{dark ? 'Dark' : 'Light'} · {mode}</h1><ChangelogView state={state} baseline={current.baseline} onRetry={() => setState({ _tag: 'Ready', manifest: fixture })} buildRevision={fixtureRevision} /></article>
}
export const AllStates: Story = {
  render: () => <div {...stylex.props(styles.matrix)}>{([true, false] as const).flatMap((dark) => (['unseen', 'seen', 'first', 'loading', 'empty', 'failure'] as const).map((mode) => <ChangelogStateCard key={`${dark}-${mode}`} mode={mode} dark={dark} />))}</div>,
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
  canvas: { position: 'relative', backgroundColor: surface.canvas, color: ink.fg, fontFamily: 'system-ui, sans-serif' },
  caption: { padding: 24, margin: 0, color: ink.fgMuted, fontSize: 13 },
  bar: { position: 'absolute', bottom: 0, left: 0, right: 0, display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: 8, padding: 12, backgroundColor: surface.raised, fontSize: 12 },
  matrix: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))', gap: 16, padding: 16, backgroundColor: surface.canvas, color: ink.fg },
  card: { backgroundColor: surface.raised, color: ink.fg, padding: 16, fontFamily: 'system-ui, sans-serif' },
  desktopBar: { position: 'fixed', bottom: 0, left: 0, right: 0, height: 32, boxSizing: 'border-box', display: 'flex', justifyContent: 'flex-end', alignItems: 'center', padding: '0 12px', backgroundColor: surface.raised, fontSize: 12 },
  dismissTarget: { padding: 24, minHeight: 100, color: ink.fgMuted },
  overflow: { position: 'fixed', inset: 'auto', bottom: 60, right: 12, margin: 0, padding: 12, backgroundColor: surface.raised, color: ink.fg },
})
