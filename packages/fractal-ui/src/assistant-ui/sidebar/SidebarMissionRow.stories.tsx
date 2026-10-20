/**
 * RM1 agent rows with mission context on line 2: `[mission pill] · step`. Designed cases cover all five mission hues, a
 * mission shared across project folders, a long slug, a question waiting on you with and without a decision handle,
 * unknown progress, an unknown step, rows without a mission and missions queued behind the current one. A separate
 * scenario story projects the catalog roster without introducing any of these fixed contract cases.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { MilestoneAgentRow } from './MilestoneAgentRow'
import { missionHues } from './SidebarMissionLine'
import { statuses, type SidebarAgentRow as Row, type SidebarMissionContext } from './model'
import { geometryVars as g, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

const storyNow = Date.UTC(2026, 9, 21, 9, 30, 0)
const base: Row = {
  ref: 'agent/base', id: 'base', title: 'Base', status: 'working', statusLabel: statuses.working.label, host: 'build-1', freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 180_000 }, children: [],
}
const mission = (id: string, slug: string, rest: Omit<Extract<SidebarMissionContext, { _tag: 'Mission' }>, '_tag' | 'id' | 'slug'>): SidebarMissionContext => ({ _tag: 'Mission', id: `mission/example/${id}`, slug, ...rest })
const row = (key: string, title: string, rest: Partial<Row>): Row => ({ ...base, ref: `agent/${key}`, id: key, title, ...rest })
const waiting = { status: 'waiting', statusLabel: statuses.waiting.label, needsMe: true } as const satisfies Partial<Row>
const longSlug = 'review-hover-scopes-and-every-long-mission-name'

/** Two project folders; `fractal-web-upstream` runs in both, so its rows must share one hue and one glyph. */
const folders: readonly { readonly label: string; readonly rows: readonly Row[] }[] = [
  { label: 'fractal-web', rows: [
    row('polish', 'Polish the sidebar rows', { missionContext: mission('sidebar-polish', 'sidebar-polish', { progress: { _tag: 'Known', done: 3, total: 7 }, step: { _tag: 'Step', name: 'Review hover scopes' }, queued: [] }) }),
    row('port', 'Port the upstream sidebar', { ...waiting, missionContext: mission('fractal-web-upstream', 'fractal-web-upstream', {
      progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Waiting', question: 'Pick hover scope', handle: 'q143' },
      queued: [{ id: 'mission/example/docs-refresh', slug: 'docs-refresh' }, { id: 'mission/example/rename-users-api', slug: 'rename-users-api' }],
    }) }),
    row('flaky', 'Chase the flaky test', { host: 'build-2', missionContext: mission('flaky-test-hunt', 'flaky-test-hunt', { progress: { _tag: 'Unknown' }, step: { _tag: 'Step', name: 'Bisect the failing run' }, queued: [] }) }),
    row('helper', 'Idle helper', { status: 'idle', statusLabel: statuses.idle.label, missionContext: { _tag: 'None' } }),
  ] },
  { label: 'smalltalk', rows: [
    row('docs', 'Refresh the API docs', { ...waiting, host: 'build-2', missionContext: mission('docs-refresh', 'docs-refresh', { progress: { _tag: 'Known', done: 1, total: 4 }, step: { _tag: 'Waiting', question: 'Approve the new page outline?' }, queued: [] }) }),
    row('rename', 'Rename the users API', { missionContext: mission('rename-users-api', 'rename-users-api', { progress: { _tag: 'Known', done: 4, total: 6 }, step: { _tag: 'Unknown' }, queued: [{ id: 'mission/example/sidebar-polish', slug: 'sidebar-polish' }] }) }),
    row('long', 'Review every long scope', { host: 'build-2', missionContext: mission('long', longSlug, { progress: { _tag: 'Known', done: 0, total: 3 }, step: { _tag: 'Step', name: 'Collect the hover scope notes for review' }, queued: [] }) }),
    row('sync', 'Sync upstream notes', { host: 'build-2', missionContext: mission('fractal-web-upstream', 'fractal-web-upstream', { progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Step', name: 'Write the port notes' }, queued: [] }) }),
    row('release', 'Release checklist', { status: 'ended', statusLabel: statuses.ended.label, host: 'build-2' }),
  ] },
]


function MissionRowsStory({ scheme, phone = false }: { readonly scheme: Scheme; readonly phone?: boolean }) {
  return <main data-testid="mission-rows" data-scheme={scheme} {...stylex.props(styles.root, phone && styles.phoneFrame, scheme === 'light' && lightTheme)}>
    <NarrowLayoutContext.Provider value={phone}><ThemePortal>
      <span aria-hidden="true" data-tone-probe="" {...stylex.props(styles.toneProbe)} />
      <div data-host="designed" {...stylex.props(styles.column, phone && styles.phoneColumn)}>
        {folders.map(folder => <section key={folder.label} aria-label={`Project ${folder.label}`} data-folder={folder.label} {...stylex.props(styles.folder)}>
          <h3 {...stylex.props(styles.heading)}>{folder.label}</h3>
          {folder.rows.map(item => <MilestoneAgentRow key={item.ref} row={item} now={storyNow} />)}
        </section>)}
      </div>
    </ThemePortal></NarrowLayoutContext.Provider>
  </main>
}


type Args = React.ComponentProps<typeof MissionRowsStory>
const meta = {
  title: 'Fractal UI/Sidebar/Mission row',
  component: MissionRowsStory,
  parameters: {
    layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Fixed mission rows verify all five hues, exact pill geometry, truncation, optional progress and question-handle states unavailable together in a catalog roster.' },
    a11y: { config: { rules: [{ id: 'label-content-name-mismatch', enabled: true }] } },
    docs: { description: { component: 'Line 2 of the RM1 row with mission context: `[mission pill] · step`. The pill is the row’s only box (16px, fully rounded, at most 56% of line 2); five stable mission hues reach only the glyph and a 14% wash. A waiting question shows its decision handle as amber text; rows without a mission read `status · host`; queued missions open from "+N" in a popover.' } },
  },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, phone: { control: 'boolean' } },
} satisfies Meta<Args>
export default meta
type Story = StoryObj<Args>

const designedRow = (canvasElement: HTMLElement, title: string) => {
  const found = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="designed"] [data-milestone-row]')].find(element => element.querySelector('[data-row-column="title"]')?.textContent === title)
  if (found === undefined) throw new Error(`Missing designed row ${title}`)
  return found
}
const line2 = (element: Element) => element.querySelector<HTMLElement>('[data-row-column="line2"]')!
const probe = (canvasElement: HTMLElement) => getComputedStyle(canvasElement.querySelector('[data-tone-probe]')!)

/** Geometry and content shared by the desktop and phone stories. */
const missionRowsPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const pills = await waitFor(() => { const found = [...canvasElement.querySelectorAll<HTMLElement>('[data-box="pill"]')]; expect(found.length).toBeGreaterThanOrEqual(7); return found })
  // Every pill starts at line 2's first pixel, so all share one x; each is 16px and fully rounded.
  const x = pills[0]!.getBoundingClientRect().left
  for (const pill of pills) {
    const box = pill.getBoundingClientRect()
    await expect(Math.abs(box.left - x)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs(box.left - line2(pill.closest('[data-milestone-row]')!).getBoundingClientRect().left)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs(box.height - 16)).toBeLessThanOrEqual(0.5)
    await expect(Number.parseFloat(getComputedStyle(pill).borderTopLeftRadius)).toBeGreaterThanOrEqual(8)
    // The mission side never takes more than 56% of line 2.
    await expect(pill.closest('[data-line2-slot="mission"]')!.getBoundingClientRect().width).toBeLessThanOrEqual(line2(pill.closest('[data-milestone-row]')!).getBoundingClientRect().width * 0.56 + 0.5)
    // The pill is the row's only box: no border, and nothing else on line 2 paints a background.
    await expect(getComputedStyle(pill).borderTopWidth).toBe('0px')
    for (const other of line2(pill.closest('[data-milestone-row]')!).querySelectorAll<HTMLElement>('[data-line2-slot="step"] *')) await expect(getComputedStyle(other).backgroundColor).toBe('rgba(0, 0, 0, 0)')
  }
  // Five hues, all present; each hue reaches only the glyph and the wash, while the slug stays on ink.
  const designed = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="designed"] [data-box="pill"]')]
  await expect(new Set(designed.map(pill => pill.dataset.missionHue))).toEqual(new Set(missionHues))
  const ink = probe(canvasElement).backgroundColor
  for (const pill of designed) {
    const glyph = pill.querySelector<SVGElement>('[data-mission-mark]')!
    await expect(getComputedStyle(pill.querySelector('[data-row-field="mission"]')!).color).toBe(ink)
    await expect(getComputedStyle(glyph).color).not.toBe(ink)
    await expect(getComputedStyle(pill).backgroundColor).not.toBe('rgba(0, 0, 0, 0)')
  }
  // One mission in two project folders: the same hue, glyph shape and wash in both.
  const [upstreamA, upstreamB] = designed.filter(pill => pill.dataset.missionPill === 'fractal-web-upstream')
  await expect(upstreamA!.closest('[data-folder]')).not.toBe(upstreamB!.closest('[data-folder]'))
  await expect(upstreamA!.dataset.missionHue).toBe(upstreamB!.dataset.missionHue)
  await expect(upstreamA!.querySelector('[data-mission-mark]')!.getAttribute('data-mission-mark')).toBe(upstreamB!.querySelector('[data-mission-mark]')!.getAttribute('data-mission-mark'))
  await expect(getComputedStyle(upstreamA!).backgroundColor).toBe(getComputedStyle(upstreamB!).backgroundColor)

  // `[pill] · step` with real n/m inside the pill.
  const polish = designedRow(canvasElement, 'Polish the sidebar rows')
  await expect(polish.querySelector('[data-mission-progress]')).toHaveTextContent('3/7')
  await expect(polish.querySelector('[data-box="pill"] [data-mission-progress]')).not.toBeNull()
  await expect(line2(polish).querySelector('[data-row-field="step"]')).toHaveTextContent('Review hover scopes')
  // A long slug truncates inside the pill; n/m stays whole and the step keeps the rest of the line.
  const long = designedRow(canvasElement, 'Review every long scope')
  const slug = long.querySelector<HTMLElement>('[data-row-field="mission"]')!
  await expect(slug.textContent).toBe(longSlug)
  await expect(slug.scrollWidth).toBeGreaterThan(slug.clientWidth)
  await expect(getComputedStyle(slug).textOverflow).toBe('ellipsis')
  const fraction = long.querySelector<HTMLElement>('[data-mission-progress]')!
  await expect(fraction.scrollWidth).toBeLessThanOrEqual(fraction.clientWidth + 0.5)
  await expect(long.querySelector('[data-line2-slot="step"]')!.getBoundingClientRect().width).toBeGreaterThan(line2(long).getBoundingClientRect().width * 0.3)
  // Unknown progress shows no n/m; an Unknown step shows the pill alone, without a separator.
  await expect(designedRow(canvasElement, 'Chase the flaky test').querySelector('[data-mission-progress]')).toBeNull()
  const rename = designedRow(canvasElement, 'Rename the users API')
  await expect(rename.querySelector('[data-line2-slot="step"]')).toBeNull()
  await expect(rename.querySelector('[data-mission-progress]')).toHaveTextContent('4/6')

  // Waiting with a handle: `[pill] · q143 Pick hover scope`; the handle is amber text, unboxed, and the row drops the "!".
  const port = designedRow(canvasElement, 'Port the upstream sidebar')
  const handle = port.querySelector<HTMLElement>('[data-q-handle]')!
  await expect(handle).toHaveTextContent('q143')
  await expect(getComputedStyle(handle).color).toBe(probe(canvasElement).color)
  await expect(getComputedStyle(handle).backgroundColor).toBe('rgba(0, 0, 0, 0)')
  await expect(getComputedStyle(handle).borderTopWidth).toBe('0px')
  await expect(handle.textContent).not.toContain('!')
  await expect(port.querySelector('[data-row-field="question"]')).toHaveTextContent('Pick hover scope')
  await expect(port.querySelector('[data-row-column="attention"]')).toBeNull()
  await expect(port.querySelector('[data-row-column="signal"]')!.textContent).toBe('')
  // Waiting without a handle shows the question alone.
  const docs = designedRow(canvasElement, 'Refresh the API docs')
  await expect(docs.querySelector('[data-q-handle]')).toBeNull()
  await expect(docs.querySelector('[data-row-field="question"]')).toHaveTextContent('Approve the new page outline?')
  await expect(docs.querySelector('[data-row-column="attention"]')).toBeNull()

  // No mission (None or absent): `status · host`, with no pill and no placeholder.
  for (const [title, text] of [['Idle helper', 'Idle·build-1'], ['Release checklist', 'Ended·build-2']] as const) {
    const plain = designedRow(canvasElement, title)
    await expect(line2(plain).textContent).toBe(text)
    await expect(plain.querySelector('[data-box], [data-line2-slot]')).toBeNull()
  }
  // Folders stay project folders: no mission grouping and no "No mission" section.
  await expect([...canvasElement.querySelectorAll('[data-host="designed"] [data-folder]')].map(folder => folder.getAttribute('data-folder'))).toEqual(['fractal-web', 'smalltalk'])
  await expect(canvasElement).not.toHaveTextContent(/no mission/i)
  for (const element of canvasElement.querySelectorAll<HTMLElement>('[data-milestone-row]')) await expect(Math.abs(element.getBoundingClientRect().height - 48)).toBeLessThanOrEqual(1)

  // The scheme reaches the surface: light renders on a light canvas, dark on a dark one.
  const surfaceRoot = within(canvasElement).getByTestId('mission-rows')
  const [red, green, blue] = getComputedStyle(surfaceRoot).backgroundColor.match(/\d+(\.\d+)?/g)!.map(Number)
  await expect((0.2126 * red! + 0.7152 * green! + 0.0722 * blue!) / 255 > 0.5).toBe(surfaceRoot.getAttribute('data-scheme') === 'light')
}

export const MissionRows: Story = { tags: ['critical'], name: 'Mission pill · every case · live default width', play: missionRowsPlay }
export const MissionRowsLight: Story = { ...MissionRows, name: 'Mission pill · every case · live default width · light', args: { scheme: 'light' } }

const queuedTrigger = (canvasElement: HTMLElement) => within(designedRow(canvasElement, 'Port the upstream sidebar')).getByRole('button', { name: '+2 more missions queued' })
const queuedDialog = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Queued missions"]')

/** "+N" opens an accessible dialog popover on press, never on hover; Escape closes it and returns focus. Ends open for the a11y check. */
const queuedPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const trigger = queuedTrigger(canvasElement)
  await expect(trigger).toHaveTextContent('+2')
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  // Plain text, not a box.
  await expect(getComputedStyle(trigger).backgroundColor).toBe('rgba(0, 0, 0, 0)')
  await expect(getComputedStyle(trigger).borderTopWidth).toBe('0px')
  // The trigger stays clear of the desktop action overlay, so the row menu never covers it.
  const menu = trigger.closest('[data-milestone-row]')!.querySelector('[data-agent-menu]')!.getBoundingClientRect()
  if (canvasElement.querySelector('[data-narrow]') === null) await expect(trigger.getBoundingClientRect().right).toBeLessThanOrEqual(menu.left + 0.5)
  // Hover alone never opens it, and it is never a tooltip.
  await userEvent.hover(trigger)
  const started = performance.now()
  do {
    await expect(queuedDialog()).toBeNull()
    await expect(document.querySelector('[role="tooltip"]')).toBeNull()
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  } while (performance.now() - started < 700)
  await userEvent.unhover(trigger)
  await userEvent.click(trigger)
  const dialog = await waitFor(() => { const element = queuedDialog(); expect(element).not.toBeNull(); expect(element).toBeVisible(); return element! })
  await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  await expect([...dialog.querySelectorAll('[data-queued-mission]')].map(item => item.getAttribute('data-queued-mission'))).toEqual(['docs-refresh', 'rename-users-api'])
  for (const item of dialog.querySelectorAll('[data-queued-mission]')) await expect(item.querySelector('[data-mission-mark]')).not.toBeNull()
  await waitFor(() => {
    const box = dialog.getBoundingClientRect()
    expect(box.left).toBeGreaterThanOrEqual(0)
    expect(box.top).toBeGreaterThanOrEqual(0)
    expect(box.right).toBeLessThanOrEqual(window.innerWidth)
    expect(box.bottom).toBeLessThanOrEqual(window.innerHeight)
  })
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(queuedDialog()).toBeNull())
  await waitFor(() => expect(trigger).toHaveFocus())
  // Keyboard opens it too; leave it open so the a11y check sees the popover.
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(queuedDialog()).toBeVisible())
}

export const QueuedMissions: Story = { tags: ['critical'], name: 'Queued missions · "+N" popover', parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed +1 and +2 mission queues prove popover geometry, keyboard focus, Escape and hover-free disclosure; catalog queues vary with work.' } }, play: queuedPlay }
export const QueuedMissionsLight: Story = { ...QueuedMissions, name: 'Queued missions · "+N" popover · light', args: { scheme: 'light' } }

/** 390px phone: the same pill geometry; "+N" keeps a 44 × 44 hit area and opens from a touch press. */
const phonePlay: NonNullable<Story['play']> = async context => {
  await missionRowsPlay(context)
  const trigger = queuedTrigger(context.canvasElement)
  await expect(Math.abs(within(context.canvasElement).getByTestId('mission-rows').getBoundingClientRect().width - 390)).toBeLessThanOrEqual(1)
  const hit = getComputedStyle(trigger, '::before')
  await expect(Number.parseFloat(hit.width)).toBeGreaterThanOrEqual(44)
  await expect(Number.parseFloat(hit.height)).toBeGreaterThanOrEqual(44)
  await userEvent.pointer([{ keys: '[TouchA>]', target: trigger }, { keys: '[/TouchA]', target: trigger }])
  const dialog = await waitFor(() => { const element = queuedDialog(); expect(element).not.toBeNull(); expect(element).toBeVisible(); return element! })
  await waitFor(() => {
    const box = dialog.getBoundingClientRect()
    expect(box.left).toBeGreaterThanOrEqual(0)
    expect(box.right).toBeLessThanOrEqual(Math.min(window.innerWidth, 390))
  })
}

export const Phone: Story = { tags: ['critical'], name: 'Phone · mission pill and "+N" touch target', parameters: { scenario: { _tag: 'contract-fixture', reason: 'A fixed 390px row matrix proves pill alignment, truncation and 44px queued-mission touch targets across every optional field case.' } }, args: { scheme: 'dark', phone: true }, play: phonePlay }
export const PhoneLight: Story = { ...Phone, name: 'Phone · mission pill and "+N" touch target · light', args: { scheme: 'light', phone: true } }


const styles = stylex.create({
  root: { display: 'flex', alignItems: 'flex-start', flexWrap: 'wrap', gap: s.xl, minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  // The live default sidebar: `sidebarDefault` wide, rows inset by `s.md` with 2px gaps.
  column: { display: 'flex', flexDirection: 'column', gap: s.xs2, width: g.sidebarDefault, boxSizing: 'border-box', padding: s.md, backgroundColor: surface.sidebar },
  folder: { display: 'flex', flexDirection: 'column', gap: s.xs2 },
  phoneFrame: { flexDirection: 'column', width: '390px', maxWidth: '100%', padding: 0, gap: 0 },
  phoneColumn: { width: '100%' },
  heading: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
  // Resolves the surface's own tokens for the plays: attention amber (decision handle) and soft ink (pill text).
  toneProbe: { position: 'absolute', width: s.zero, height: s.zero, overflow: 'hidden', color: tone.attention, backgroundColor: ink.fgSoft },
})
