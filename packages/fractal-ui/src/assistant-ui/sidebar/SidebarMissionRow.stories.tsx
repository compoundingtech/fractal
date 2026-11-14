/**
 * RM1 agent rows with mission context on line 2: `ring n/m · step · slug`. Designed cases cover neutral ink, a
 * mission shared across project folders, a long slug, a question waiting on you with and without a decision handle,
 * unknown progress, an unknown step, rows without a mission, missions queued behind the current one and retained Stale
 * (with and without a reported update time) and Failed mission info. The phone frame proves the merged 44px mission
 * target on a touch layout. The ETA stories prove a served range or single value adds only its hidden description and the
 * row's accessible name, never a drawn slot, so every ETA state lays out identically; null, absent or finished ETAs add
 * nothing. A separate scenario story projects the catalog roster without these fixed contract cases.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { MilestoneAgentRow } from './MilestoneAgentRow'
import { SidebarTree, SidebarTreeDisclosure, type SidebarTreeItem } from './SidebarTree'
import type { Key } from 'react-aria-components'
import { SidebarMissionChildRow, missionChildAccessibleName } from './SidebarMissionLine'
import { statuses, type SidebarAgentRow as Row, type SidebarMissionContext, type SidebarOtherMission } from './model'
import type { MissionEta } from '../MissionEta'
import { sidebarRowAccessibleName } from './rowPresentation.tsx'
import { geometryVars as g, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { touchPointerQuery } from './useTouchLayout'

const storyNow = Date.UTC(2026, 9, 21, 9, 30, 0)
const base: Row = {
  ref: 'agent/example-base', id: 'base', title: 'Base', status: 'working', statusLabel: statuses.working.label, host: 'build-1', freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 180_000 }, children: [],
}
const mission = (id: string, slug: string, rest: Omit<Extract<SidebarMissionContext, { _tag: 'Mission' }>, '_tag' | 'id' | 'slug'>): SidebarMissionContext => ({ _tag: 'Mission', id: `mission/example/${id}`, slug, ...rest })
const row = (key: string, title: string, rest: Partial<Row>): Row => ({ ...base, ref: `agent/example-${key}`, id: key, title, ...rest })
const waiting = { status: 'waiting', statusLabel: statuses.waiting.label, needsMe: true } as const satisfies Partial<Row>
const longSlug = 'review-hover-scopes-and-every-long-mission-name'
const staleAt = storyNow - 600_000

/** Two project folders; `fractal-web-upstream` runs in both, so its rows read the same in both. */
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
  { label: 'fractal', rows: [
    row('docs', 'Refresh the API docs', { ...waiting, host: 'build-2', missionContext: mission('docs-refresh', 'docs-refresh', { progress: { _tag: 'Known', done: 1, total: 4 }, step: { _tag: 'Waiting', question: 'Approve the new page outline?' }, queued: [] }) }),
    row('rename', 'Rename the users API', { missionContext: mission('rename-users-api', 'rename-users-api', { progress: { _tag: 'Known', done: 4, total: 6 }, step: { _tag: 'Unknown' }, queued: [{ id: 'mission/example/sidebar-polish', slug: 'sidebar-polish' }] }) }),
    row('long', 'Review every long scope', { host: 'build-2', missionContext: mission('long', longSlug, { progress: { _tag: 'Known', done: 0, total: 3 }, step: { _tag: 'Step', name: 'Collect the hover scope notes for review' }, queued: [] }) }),
    row('sync', 'Sync upstream notes', { host: 'build-2', missionContext: mission('fractal-web-upstream', 'fractal-web-upstream', { progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Step', name: 'Write the port notes' }, queued: [] }) }),
    row('release', 'Release checklist', { status: 'ended', statusLabel: statuses.ended.label, host: 'build-2' }),
    row('unavailable', 'Mission feed failed before a read', { missionContext: { _tag: 'Unavailable' }, missionInfo: { _tag: 'Failed' } }),
    // Retained mission info: Stale with a reported update time, Stale without one, and Failed (no time) with a queue.
    row('uploads', 'Watch the upload retries', { missionInfo: { _tag: 'Stale', updatedAt: staleAt }, missionContext: mission('upload-retries', 'upload-retries', { progress: { _tag: 'Known', done: 1, total: 3 }, step: { _tag: 'Step', name: 'Watch the next upload' }, queued: [] }) }),
    row('search', 'Index the search logs', { host: 'build-2', missionInfo: { _tag: 'Stale' }, missionContext: mission('search-index', 'search-index', { progress: { _tag: 'Unknown' }, step: { _tag: 'Step', name: 'Rebuild the log index' }, queued: [] }) }),
    row('billing', 'Map the billing events', { ...waiting, host: 'build-2', missionInfo: { _tag: 'Failed' }, missionContext: mission('billing-events', 'billing-events', {
      progress: { _tag: 'Known', done: 2, total: 4 }, step: { _tag: 'Waiting', question: 'Keep the legacy event names?', handle: 'q207' }, queued: [{ id: 'mission/example/docs-refresh', slug: 'docs-refresh' }],
    }) }),
  ] },
]

interface MissionTreeItem extends SidebarTreeItem<MissionTreeItem> { readonly row: Row }
const missionChild: Row = { ...folders[0]!.rows[1]!, ref: 'agent/example-mission-helper', parentRef: 'agent/example-mission-parent', title: 'Observed mission helper', children: [] }
const missionParent: Row = { ...folders[1]!.rows[3]!, ref: 'agent/example-mission-parent', title: 'Mission parent with subagents', children: [missionChild],
  missionContext: mission('fractal-web-upstream', 'fractal-web-upstream', { progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Step', name: 'Write the port notes' }, queued: [{ id: 'mission/example/sidebar-polish', slug: 'sidebar-polish' }] }),
}
const missionTree: readonly MissionTreeItem[] = [{
  id: missionParent.ref, depth: 0, textValue: missionParent.title, accessibleName: sidebarRowAccessibleName(missionParent), row: missionParent, disclosurePlacement: 'trailing',
  children: [{ id: missionChild.ref, depth: 1, textValue: missionChild.title, row: missionChild, children: [], isDisabled: true }],
}]
/**
 * A parent pinned above enough helpers to scroll its real row out of the virtualized tree. Like a live sidebar row bound
 * to its own data, its queue comes from context: the tree's items and `renderRow` keep their identity when it changes.
 */
const stickyHelpers: readonly Row[] = Array.from({ length: 12 }, (_, index) => ({ ...missionChild, ref: `agent/example-sticky-helper-${index + 1}`, parentRef: 'agent/example-sticky-parent', title: `Pinned parent helper ${index + 1}` }))
const stickyParent: Row = { ...missionParent, ref: 'agent/example-sticky-parent', title: 'Mission parent pinned above helpers', children: stickyHelpers,
  missionContext: mission('fractal-web-upstream', 'fractal-web-upstream', { progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Step', name: 'Write the port notes' }, queued: [] }),
}
const stickyParentQueued: Row = { ...stickyParent, missionContext: missionParent.missionContext }
const stickyTree: readonly MissionTreeItem[] = [{
  id: stickyParent.ref, depth: 0, textValue: stickyParent.title, accessibleName: sidebarRowAccessibleName(stickyParent), row: stickyParent, disclosurePlacement: 'trailing',
  children: stickyHelpers.map(helper => ({ id: helper.ref, depth: 1, textValue: helper.title, row: helper, children: [], isDisabled: true })),
}]
const StickyQueueContext = React.createContext(false)
function StickyMissionRow({ item, expanded, trailingDisclosure }: { readonly item: MissionTreeItem; readonly expanded: boolean; readonly trailingDisclosure: boolean }) {
  const queued = React.useContext(StickyQueueContext)
  return <MilestoneAgentRow row={queued && item.row === stickyParent ? stickyParentQueued : item.row} now={storyNow} displayOnly={item.isDisabled} active={item.isDisabled}
    touchDisclosure={trailingDisclosure ? <SidebarTreeDisclosure expanded={expanded} /> : undefined} />
}
const renderStickyRow = (item: MissionTreeItem, state: { readonly expanded: boolean; readonly trailingDisclosure: boolean }) =>
  <StickyMissionRow item={item} expanded={state.expanded} trailingDisclosure={state.trailingDisclosure} />


function MissionParentStory() {
  const [expanded, setExpanded] = React.useState<Set<Key>>(() => new Set([missionParent.ref]))
  const [opened, setOpened] = React.useState<string>()
  const [stickyExpanded, setStickyExpanded] = React.useState<Set<Key>>(() => new Set([stickyParent.ref]))
  const [queued, setQueued] = React.useState(false)
  return <>
    <section aria-label="Mission parent" data-host="mission-parent" data-opened={opened} {...stylex.props(styles.column, styles.phoneColumn, styles.parentViewport)}>
      <SidebarTree aria-label="Mission parent and helper" items={missionTree} expandedKeys={expanded} onExpandedChange={setExpanded}
        onAction={key => setOpened(String(key))}
        renderRow={(item, state) => <MilestoneAgentRow row={item.row} now={storyNow} displayOnly={item.isDisabled} active={item.isDisabled}
          onOpen={item.isDisabled ? undefined : row => setOpened(row.ref)}
          touchDisclosure={state.trailingDisclosure ? <SidebarTreeDisclosure expanded={state.expanded} /> : undefined} />} />
    </section>
    <section aria-label="Pinned mission parent" data-host="sticky-parent" {...stylex.props(styles.column, styles.phoneColumn)}>
      <button type="button" aria-pressed={queued} onClick={() => setQueued(value => !value)} {...stylex.props(styles.queueToggle)}>Queue a mission</button>
      <div {...stylex.props(styles.stickyViewport)}>
        <StickyQueueContext.Provider value={queued}>
          <SidebarTree aria-label="Pinned mission parent and helpers" items={stickyTree} expandedKeys={stickyExpanded} onExpandedChange={setStickyExpanded} renderRow={renderStickyRow} />
        </StickyQueueContext.Provider>
      </div>
    </section>
  </>
}


function MissionRowsStory({ scheme, phone = false, queueLifecycle = false }: { readonly scheme: Scheme; readonly phone?: boolean; readonly queueLifecycle?: boolean }) {
  const [drained, setDrained] = React.useState(false)
  return <main data-testid="mission-rows" data-scheme={scheme} {...stylex.props(styles.root, phone && styles.phoneFrame, scheme === 'light' && lightTheme)}>
    <NarrowLayoutContext.Provider value={phone}><ThemePortal>
      <span aria-hidden="true" data-tone-probe="" {...stylex.props(styles.toneProbe)} />
      {queueLifecycle && <button type="button" onClick={() => setDrained(value => !value)}>{drained ? 'Restore queue' : 'Drain queue'}</button>}
      <div data-host="designed" {...stylex.props(styles.column, phone && styles.phoneColumn)}>
        {folders.map(folder => <section key={folder.label} aria-label={`Project ${folder.label}`} data-folder={folder.label} {...stylex.props(styles.folder)}>
          <h3 {...stylex.props(styles.heading)}>{folder.label}</h3>
          {folder.rows.map(item => <MilestoneAgentRow key={item.ref} row={drained && item.title === 'Port the upstream sidebar' && item.missionContext?._tag === 'Mission' ? { ...item, missionContext: { ...item.missionContext, queued: [] } } : item} now={storyNow} />)}
        </section>)}
      </div>
      {phone && <MissionParentStory />}
    </ThemePortal></NarrowLayoutContext.Provider>
  </main>
}


type Args = React.ComponentProps<typeof MissionRowsStory>
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Sidebar/Mission row',
  component: MissionRowsStory,
  parameters: {
    layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Fixed mission rows verify neutral ink, line geometry, truncation, optional progress, question-handle states and retained Stale/Failed mission info unavailable together in a catalog roster.' },
    a11y: { config: { rules: [{ id: 'label-content-name-mismatch', enabled: true }] } },
    docs: { description: { component: 'Line 2 of the RM1 row with mission context: `ring n/m · step · slug`. The 12px ring is quiet and neutral, the slug is muted and gives up its width first, and ad hoc work reads `title ?? goal` with no ring and no hash. A waiting question shows its decision handle as amber text; rows without a mission read `status · host`; queued missions open from "+N" in a popover. Stale or Failed mission info mutes the mission and opens a "Mission info" popover from the pill. On touch layouts the pill and "+N" are one button filling a 44px line-2 track.' } },
  },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, phone: { control: 'boolean' } },
} satisfies Meta<Args>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'ChildRows:contract-fixture',
  'EtaAbsent:contract-fixture',
  'EtaFinishing:contract-fixture',
  'EtaNull:contract-fixture',
  'EtaRange:contract-fixture',
  'EtaSingle:contract-fixture',
  'EtaStates:contract-fixture',
  'EtaStatesLight:contract-fixture',
  'MissionRows:contract-fixture',
  'MissionRowsLight:contract-fixture',
  'OpensMission:contract-fixture',
  'Phone:contract-fixture',
  'PhoneLight:contract-fixture',
  'QueuedMissions:contract-fixture',
  'QueuedMissionsLight:contract-fixture',
]
type Story = StoryObj<Args>

const designedRow = (canvasElement: HTMLElement, title: string) => {
  const found = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="designed"] [data-milestone-row]')].find(element => element.querySelector('[data-row-column="title"]')?.textContent === title)
  if (found === undefined) throw new Error(`Missing designed row ${title}`)
  return found
}
const line2 = (element: Element) => element.querySelector<HTMLElement>('[data-row-column="line2"]')!
const probe = (canvasElement: HTMLElement) => getComputedStyle(canvasElement.querySelector('[data-tone-probe]')!)
const rect = (element: Element) => element.getBoundingClientRect()
const intersects = (a: DOMRect, b: DOMRect) => a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5
const infoDialog = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Mission info"]')
/** The mission button's content-derived name, comma-joined parts; tolerant only of the whitespace name computation adds around inline-flex boxes. */
const missionName = (...parts: readonly string[]) => {
  const escaped = parts.map(part => part.replace(/[.*+?^$()|[\]\\{}]/g, '\\$&').replace(/ /g, '\\s+'))
  return new RegExp('^' + escaped.join('\\s*,\\s*') + '$')
}
const retainedCases = [
  { title: 'Watch the upload retries', info: 'stale', updatedAt: staleAt, note: 'Mission info may be out of date. Updated 10m ago.', name: ['1 of 3 steps'], touchQueue: [], reason: 'mission info may be out of date' },
  { title: 'Index the search logs', info: 'stale', updatedAt: undefined, note: 'Mission info may be out of date.', name: [], touchQueue: [], reason: 'mission info may be out of date' },
  { title: 'Map the billing events', info: 'failed', updatedAt: undefined, note: 'Couldn’t refresh mission info.', name: ['2 of 4 steps'], touchQueue: ['+1 more mission queued'], reason: 'couldn’t refresh mission info' },
] as const

/** Geometry and content shared by the desktop and phone stories. */
const missionRowsPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const marks = await waitFor(() => { const found = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="designed"] [data-mission-ring]')]; expect(found.length).toBeGreaterThanOrEqual(7); return found })
  // Bare rings align with line 2; phone disclosure rings sit inside their 44px touch target.
  for (const mark of marks) {
    const box = mark.getBoundingClientRect()
    const row = mark.closest('[data-milestone-row]')!
    const touchTarget = mark.closest<HTMLElement>('button[data-mission-trigger]')
    const lineLeft = line2(row).getBoundingClientRect().left
    if (row.hasAttribute('data-touch-mission') && touchTarget) {
      await expect(touchTarget.getBoundingClientRect().left).toBeCloseTo(lineLeft, 0)
      await expect(box.left).toBeGreaterThanOrEqual(touchTarget.getBoundingClientRect().left)
      await expect(box.right).toBeLessThanOrEqual(touchTarget.getBoundingClientRect().right)
    } else {
      await expect(Math.abs(box.left - lineLeft)).toBeLessThanOrEqual(0.5)
    }
    await expect(Math.abs(box.height - 12)).toBeLessThanOrEqual(0.5)
    await expect(Math.abs(box.width - 12)).toBeLessThanOrEqual(0.5)
  }
  // Nothing on line 2 paints a box or takes a hue: the slug is muted ink and the ring inherits the line's neutral.
  const designed = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="designed"] [data-milestone-row]')]
  const ink = probe(canvasElement).backgroundColor
  for (const element of designed) {
    for (const other of line2(element).querySelectorAll<HTMLElement>('[data-line2-slot] *')) await expect(getComputedStyle(other).backgroundColor).toBe('rgba(0, 0, 0, 0)')
    if (line2(element).getAttribute('data-mission-info') === 'live') for (const mark of element.querySelectorAll('[data-mission-ring]')) await expect(getComputedStyle(mark).color).toBe(ink)
  }
  // One mission in two project folders reads the same in both.
  const [upstreamA, upstreamB] = [...canvasElement.querySelectorAll<HTMLElement>('[data-host="designed"] [data-mission="fractal-web-upstream"]')]
  await expect(upstreamA!.closest('[data-folder]')).not.toBe(upstreamB!.closest('[data-folder]'))
  await expect(getComputedStyle(upstreamA!).color).toBe(getComputedStyle(upstreamB!).color)

  // `ring n/m · step · slug`.
  const polish = designedRow(canvasElement, 'Polish the sidebar rows')
  await expect(polish.querySelector('[data-mission-progress]')).toHaveTextContent('3/7')
  await expect(polish.querySelector('[data-line2-slot="mission"] [data-mission-progress]')).not.toBeNull()
  await expect(line2(polish).querySelector('[data-row-field="step"]')).toHaveTextContent('Review hover scopes')
  // A long slug gives up its width first; n/m stays whole and the step keeps the rest of the line.
  const long = designedRow(canvasElement, 'Review every long scope')
  const slug = long.querySelector<HTMLElement>('[data-row-field="mission"]')!
  await expect(slug.textContent).toBe(longSlug)
  await expect(getComputedStyle(slug).textOverflow).toBe('ellipsis')
  const fraction = long.querySelector<HTMLElement>('[data-mission-progress]')!
  await expect(fraction.scrollWidth).toBeLessThanOrEqual(fraction.clientWidth + 0.5)
  await expect(long.querySelector('[data-line2-slot="step"]')!.getBoundingClientRect().width).toBeGreaterThan(line2(long).getBoundingClientRect().width * 0.3)
  // Unknown progress shows no n/m; an Unknown step shows the ring and slug alone.
  await expect(designedRow(canvasElement, 'Chase the flaky test').querySelector('[data-mission-progress]')).toBeNull()
  const rename = designedRow(canvasElement, 'Rename the users API')
  await expect(rename.querySelector('[data-line2-slot="step"]')).toBeNull()
  await expect(rename.querySelector('[data-mission-progress]')).toHaveTextContent('4/6')

  // Waiting with a handle on a real ask: `ring n/m · q143 Pick hover scope`; the handle is amber text, unboxed, beside the amber disc.
  const port = designedRow(canvasElement, 'Port the upstream sidebar')
  const handle = port.querySelector<HTMLElement>('[data-q-handle]')!
  await expect(handle).toHaveTextContent('q143')
  await expect(getComputedStyle(handle).color).toBe(probe(canvasElement).color)
  await expect(getComputedStyle(handle).backgroundColor).toBe('rgba(0, 0, 0, 0)')
  await expect(getComputedStyle(handle).borderTopWidth).toBe('0px')
  await expect(handle.textContent).not.toContain('!')
  await expect(port.querySelector('[data-row-field="question"]')).toHaveTextContent('Pick hover scope')
  await expect(port.querySelector('[data-row-column="attention"]')).toBeNull()
  // An empty signal reserves no slot at all.
  await expect(port.querySelector('[data-row-column="signal"]')).toBeNull()
  // Waiting without a handle shows the question alone.
  const docs = designedRow(canvasElement, 'Refresh the API docs')
  await expect(docs.querySelector('[data-q-handle]')).toBeNull()
  await expect(docs.querySelector('[data-row-field="question"]')).toHaveTextContent('Approve the new page outline?')
  await expect(docs.querySelector('[data-row-column="attention"]')).toBeNull()

  // No mission (None or absent): `status · host`, with no ring and no placeholder.
  for (const [title, text] of [['Idle helper', 'Idle·build-1'], ['Release checklist', 'Ended·build-2']] as const) {
    const plain = designedRow(canvasElement, title)
    await expect(line2(plain).textContent).toBe(text)
    await expect(plain.querySelector('[data-mission-ring], [data-line2-slot]')).toBeNull()
  }
  // Failed before any value is not the same as an agent without a mission.
  const unavailable = designedRow(canvasElement, 'Mission feed failed before a read')
  const warning = unavailable.querySelector<HTMLElement>('[data-mission-unavailable]')!
  await expect(warning).toHaveTextContent('Mission info unavailable')
  await expect(warning.querySelector('svg')).toHaveAttribute('aria-hidden', 'true')
  await expect(getComputedStyle(warning.querySelector('svg')!).display).toBe('inline-block')
  await expect(rect(warning).height).toBeLessThanOrEqual(rect(line2(unavailable)).height)
  await expect(warning.closest('button')).toBeNull()
  await expect(unavailable.querySelector('[data-mission], [data-mission-trigger], [data-queued-missions]')).toBeNull()
  await expect(line2(unavailable).textContent).toBe(' Mission info unavailable·build-1')
  await expect(getComputedStyle(warning).color).toBe(getComputedStyle(designedRow(canvasElement, 'Watch the upload retries').querySelector('[data-row-field="mission"]')!).color)
  // Folders stay project folders: no mission grouping and no "No mission" section.
  await expect([...canvasElement.querySelectorAll('[data-host="designed"] [data-folder]')].map(folder => folder.getAttribute('data-folder'))).toEqual(['fractal-web', 'fractal'])
  await expect(canvasElement).not.toHaveTextContent(/no mission/i)
  // Desktop rows stay 48px; only a touch row whose mission line holds a button grows its line 2 to the 44px track.
  const touch = canvasElement.querySelector('[data-narrow]') !== null || window.matchMedia(touchPointerQuery).matches
  if (!touch) await expect(canvasElement.querySelector('[data-touch-mission]')).toBeNull()
  for (const element of canvasElement.querySelectorAll<HTMLElement>('[data-milestone-row]')) await expect(Math.abs(rect(element).height - (element.hasAttribute('data-touch-mission') ? 68 : 48))).toBeLessThanOrEqual(1)

  // Retained mission info: a dashed ring and muted text, its real update time only when reported, and a
  // "Mission info" popover from the ring even with nothing queued. No retry: the host offers none.
  for (const retained of retainedCases) {
    const element = designedRow(canvasElement, retained.title)
    const slot = element.querySelector<HTMLElement>('[data-line2-slot="mission"]')!
    await expect(line2(element)).toHaveAttribute('data-mission-info', retained.info)
    await expect(slot).toHaveAttribute('data-mission-info', retained.info)
    await expect(slot.getAttribute('data-mission-updated-at')).toBe(retained.updatedAt === undefined ? null : String(retained.updatedAt))
    const slugColor = getComputedStyle(element.querySelector('[data-row-field="mission"]')!).color
    await expect(slugColor).not.toBe(ink)
    await expect(getComputedStyle(slot).color).toBe(slugColor)
    await expect(getComputedStyle(element.querySelector('[data-line2-slot="step"]')!).color).toBe(slugColor)
    const handle = element.querySelector<HTMLElement>('[data-q-handle]')
    if (handle !== null) await expect(getComputedStyle(handle).color).not.toBe(probe(canvasElement).color)
    const trigger = element.querySelector<HTMLElement>('button[data-mission-trigger="info"]')!
    await expect(trigger).toContainElement(element.querySelector('[data-mission-ring]'))
    // The queue belongs to the mission's one target on every pointer type.
    await expect(trigger).toHaveAccessibleName(missionName(...retained.name, ...retained.touchQueue, retained.reason))
    await userEvent.click(trigger)
    const dialog = await waitFor(() => { const found = infoDialog(); expect(found).not.toBeNull(); expect(found).toBeVisible(); return found! })
    await expect(dialog.querySelector('[data-mission-info-note]')!.textContent).toBe(retained.note)
    await expect(dialog.querySelectorAll('time')).toHaveLength(retained.updatedAt === undefined ? 0 : 1)
    await expect(dialog).not.toHaveTextContent('just now')
    if (retained.updatedAt !== undefined) await expect(dialog.querySelector('time')).toHaveAttribute('datetime', new Date(retained.updatedAt).toISOString())
    await expect(dialog.querySelector('button')).toBeNull()
    await waitFor(() => expect(dialog).toHaveFocus())
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(infoDialog()).toBeNull())
    await waitFor(() => expect(trigger).toHaveFocus())
  }

  // The scheme reaches the surface: light renders on a light canvas, dark on a dark one.
  const surfaceRoot = within(canvasElement).getByTestId('mission-rows')
  const [red, green, blue] = getComputedStyle(surfaceRoot).backgroundColor.match(/\d+(\.\d+)?/g)!.map(Number)
  await expect((0.2126 * red! + 0.7152 * green! + 0.0722 * blue!) / 255 > 0.5).toBe(surfaceRoot.getAttribute('data-scheme') === 'light')
}

export const MissionRows: Story = { tags: ['critical'], name: 'Mission line · every case · live default width', play: missionRowsPlay }
export const MissionRowsLight: Story = { ...MissionRows, name: 'Mission line · every case · live default width · light', args: { scheme: 'light' } }

const queuedTrigger = (canvasElement: HTMLElement) => within(designedRow(canvasElement, 'Port the upstream sidebar')).getByRole('button', { name: missionName('+2 more missions queued') })
const queuedDialog = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Queued missions"]')

/** "+N" opens an accessible dialog popover on press, never on hover; Escape closes it and returns focus. Ends open for the a11y check. */
const queuedPlay: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const drain = within(canvasElement).getByRole('button', { name: 'Drain queue' })
  const trigger = queuedTrigger(canvasElement)
  await expect(trigger).toHaveTextContent('+2')
  await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  const missionRow = trigger.closest('[data-milestone-row]')!
  // The line reads `ring n/m · step · slug · +N`.
  await expect(rect(trigger).left).toBeGreaterThanOrEqual(rect(missionRow.querySelector('[data-line2-slot="step"]')!).right)
  // Plain text, not a box.
  await expect(getComputedStyle(trigger).backgroundColor).toBe('rgba(0, 0, 0, 0)')
  await expect(getComputedStyle(trigger).borderTopWidth).toBe('0px')
  // The trigger stays clear of the desktop action overlay, so the row menu never covers it.
  const menu = trigger.closest('[data-milestone-row]')!.querySelector('[data-agent-menu]')!.getBoundingClientRect()
  if (canvasElement.querySelector('[data-narrow]') === null) await expect(trigger.getBoundingClientRect().right).toBeLessThanOrEqual(menu.left + 0.5)
  // Hover alone never opens the queue; the row may independently show its agent hover card.
  trigger.focus()
  await expect(trigger).toHaveFocus()
  await userEvent.hover(trigger)
  const started = performance.now()
  do {
    await expect(queuedDialog()).toBeNull()
    await expect(document.querySelector('[role="tooltip"] [data-queued-mission]')).toBeNull()
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  } while (performance.now() - started < 700)
  // Opening the queue must dismiss the pointer-open passive card before the dialog owns Escape.
  if (!window.matchMedia(touchPointerQuery).matches) {
    await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).not.toBeNull())
  }
  // RAC's keyboard press need not dispatch a click or focus event on this already-focused trigger.
  await userEvent.keyboard(' ')
  const dialog = await waitFor(() => { const element = queuedDialog(); expect(element).not.toBeNull(); expect(element).toBeVisible(); return element! })
  await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull())
  await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  await expect([...dialog.querySelectorAll('[data-queued-mission]')].map(item => item.getAttribute('data-queued-mission'))).toEqual(['docs-refresh', 'rename-users-api'])
  await waitFor(() => {
    const box = dialog.getBoundingClientRect()
    expect(box.left).toBeGreaterThanOrEqual(0)
    expect(box.top).toBeGreaterThanOrEqual(0)
    expect(box.right).toBeLessThanOrEqual(window.innerWidth)
    expect(box.bottom).toBeLessThanOrEqual(window.innerHeight)
  })
  await waitFor(() => expect(dialog).toHaveFocus())
  await userEvent.unhover(trigger)
  await userEvent.hover(missionRow)
  const reentered = performance.now()
  do {
    await expect(document.querySelector('[data-testid="agent-hover-card"]')).toBeNull()
    await new Promise<void>((resolve: () => void) => requestAnimationFrame(() => resolve()))
  } while (performance.now() - reentered < 700)
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(queuedDialog()).toBeNull())
  await waitFor(() => expect(trigger).toHaveFocus())
  // Keyboard opens it too, before a live-data update removes the open trigger.
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(queuedDialog()).toBeVisible())
  // Invoke the state update without an outside pointer press that would dismiss the dialog first.
  drain.click()
  await waitFor(() => expect(queuedDialog()).toBeNull())
  await waitFor(() => expect(missionRow.querySelector('[data-mission-trigger]')).toBeNull())
  await userEvent.unhover(missionRow)
  await userEvent.hover(missionRow)
  if (!window.matchMedia(touchPointerQuery).matches) {
    await waitFor(() => expect(document.querySelector('[data-testid="agent-hover-card"]')).not.toBeNull())
  }
  within(canvasElement).getByRole('button', { name: 'Restore queue' }).click()
  const restored = await waitFor(() => queuedTrigger(canvasElement))
  await userEvent.click(restored)
  await waitFor(() => expect(queuedDialog()).toBeVisible())
}

/** Further current missions are their own 48px tree rows under the agent, with the same line and no hash. */
const childMissions: readonly SidebarOtherMission[] = [
  { id: 'mission/example/billing-cutover', slug: 'billing-cutover', title: 'Billing cutover', progress: { _tag: 'Known', done: 1, total: 4 }, step: { _tag: 'Step', name: 'Backfill empty rows' } },
  { id: 'mission/person-ask/d45524b5ff5388299cd145fd8bef8c7e', slug: 'd45524b5ff5388299cd145fd8bef8c7e', title: 'Ask about the release', progress: { _tag: 'Unknown' }, step: { _tag: 'Step', name: 'Ask about the release' } },
]
export const ChildRows: Story = {
  name: 'Mission child rows',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed named and ad hoc child rows verify the 48px height and that a hash is neither drawn nor spoken.' } },
  render: ({ scheme }) => <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><div {...stylex.props(styles.column)}>{childMissions.map(item => <SidebarMissionChildRow key={item.id} mission={item} />)}</div></main>,
  play: async ({ canvasElement }) => {
    const rows = [...canvasElement.querySelectorAll<HTMLElement>('[data-mission-child]')]
    await expect(rows).toHaveLength(2)
    for (const element of rows) await expect(Math.abs(rect(element).height - 48)).toBeLessThanOrEqual(1)
    await expect(rows[0]!.querySelector('[data-mission-progress]')).toHaveTextContent('1/4')
    await expect(rows[0]!.querySelector('[data-mission]')).toHaveTextContent('billing-cutover')
    await expect(canvasElement.textContent).not.toMatch(/[0-9a-f]{16,}/)
    await expect(missionChildAccessibleName(childMissions[1]!)).toBe('Ask about the release, Mission')
  },
}

/** The mission run of a line is its own target: pressing it opens the mission, not the agent, and hover shows its box. */
export const OpensMission: Story = {
  name: 'Mission run opens the mission',
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'A fixed tree row verifies the mission press reaches the mission callback and never the tree item, by pointer and keyboard.' } },
  render: ({ scheme }) => {
    const [log, setLog] = React.useState<readonly string[]>([])
    const missionRow = row('opens', 'Opens its mission', { mission: 'mission/example/rename', missionContext: mission('rename', 'rename', { progress: { _tag: 'Known', done: 3, total: 5 }, step: { _tag: 'Step', name: 'Rename call sites' }, queued: [] }) })
    const items = [{ id: missionRow.ref, textValue: missionRow.title, depth: 0, children: [] }]
    return <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}><div {...stylex.props(styles.column)}>
      <SidebarTree aria-label="Agents" items={items} expandedKeys={new Set<Key>()} onExpandedChange={() => {}} onAction={key => setLog(value => [...value, `agent:${String(key)}`])} renderRow={() => <MilestoneAgentRow row={missionRow} now={storyNow} onOpenMission={ref => setLog(value => [...value, `mission:${ref}`])} />} />
      <output data-testid="open-log">{log.join(' ')}</output>
    </div></main>
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const open = await canvas.findByRole('button', { name: /^Open mission/ })
    await expect(open).toHaveAccessibleName(expect.stringContaining('Rename call sites'))
    await userEvent.click(open)
    await expect(canvas.getByTestId('open-log')).toHaveTextContent('mission:mission/example/rename')
    await expect(canvas.getByTestId('open-log')).not.toHaveTextContent('agent:')
    await userEvent.hover(open)
    await waitFor(() => expect(getComputedStyle(open).backgroundColor).not.toBe('rgba(0, 0, 0, 0)'))
    await userEvent.click(canvasElement.querySelector('[data-row-column="title"]')!)
    await expect(canvas.getByTestId('open-log')).toHaveTextContent('agent:')
  },
}

export const QueuedMissions: Story = { tags: ['critical'], name: 'Queued missions · "+N" popover', parameters: { scenario: { _tag: 'contract-fixture', reason: 'Fixed +1 and +2 mission queues prove popover geometry, keyboard focus, Escape, hover-free disclosure and preview recovery when live data drains an open queue; catalog queues vary with work.' } }, render: ({ scheme }) => <MissionRowsStory scheme={scheme} queueLifecycle />, play: queuedPlay }
export const QueuedMissionsLight: Story = { ...QueuedMissions, name: 'Queued missions · "+N" popover · light', args: { scheme: 'light' } }

const etaAsOf = storyNow - 120_000
const etaRange: MissionEta = { remaining_low_ms: 720_000, remaining_high_ms: 1_080_000, basis: { method: 'critical-path', samples: 12 }, as_of: etaAsOf }
const etaStep = { progress: { _tag: 'Known', done: 2, total: 5 }, step: { _tag: 'Step', name: 'Review hover scopes' }, queued: [] } as const
/**
 * Served range, served single value, served null, an older daemon that omits it, and a finished run that must hide it.
 * One slug and step throughout, so their line 2 geometry must match exactly whatever the ETA state.
 */
const etaRows: readonly Row[] = [
  row('eta-range', 'ETA range', { missionContext: mission('eta-range', 'sidebar-polish', { ...etaStep, eta: etaRange }) }),
  row('eta-single', 'ETA single', { missionContext: mission('eta-single', 'sidebar-polish', { ...etaStep, eta: { ...etaRange, remaining_high_ms: 720_000, basis: { method: 'per-step', samples: 1 } } }) }),
  row('eta-finishing', 'ETA finishing', { missionContext: mission('eta-finishing', 'sidebar-polish', { ...etaStep, eta: { ...etaRange, remaining_low_ms: 0, remaining_high_ms: 0, basis: { method: 'critical-path', samples: 0 } } }) }),
  row('eta-null', 'ETA null', { missionContext: mission('eta-null', 'sidebar-polish', { ...etaStep, eta: null }) }),
  row('eta-absent', 'ETA absent', { missionContext: mission('eta-absent', 'sidebar-polish', etaStep) }),
  row('eta-finished', 'ETA finished', { missionContext: mission('eta-finished', 'sidebar-polish', { ...etaStep, progress: { _tag: 'Known', done: 5, total: 5 }, eta: null }) }),
]
function EtaStatesStory({ scheme, only }: { readonly scheme: Scheme; readonly only?: string }) {
  return <main data-testid="mission-eta-rows" data-scheme={scheme} {...stylex.props(styles.root, scheme === 'light' && lightTheme)}>
    <ThemePortal><div data-host="designed" {...stylex.props(styles.column)}>{etaRows.filter(item => only === undefined || item.title === only).map(item => <MilestoneAgentRow key={item.ref} row={item} now={storyNow} />)}</div></ThemePortal>
  </main>
}

/** Line 2 as drawn, relative to its row: `ring n/m · step` with the step keeping a readable width. */
const line2Geometry = (row: HTMLElement) => {
  const origin = rect(row)
  const box = (selector: string) => { const found = rect(row.querySelector(selector)!); return [found.left - origin.left, found.top - origin.top, found.width, found.height].map(Math.round) }
  return { row: [Math.round(origin.width), Math.round(origin.height)], mission: box('[data-line2-slot="mission"]'), ring: box('[data-mission-ring]'), step: box('[data-line2-slot="step"]') }
}
const expectDrawnLine = async (row: HTMLElement) => {
  await waitFor(() => expect(row.querySelector('[data-line2-slot="step"]')).not.toBeNull())
  await expect(row.querySelector('[data-line2-slot="eta"]')).toBeNull()
  // The step stays readable: at least "Review h…" at the narrowest sidebar.
  await expect(rect(row.querySelector('[data-row-field="step"]')!).width).toBeGreaterThanOrEqual(56)
}
/** A served ETA: only a visually hidden description in the mission slot, plus the row's accessible name; nothing drawn. */
const expectEta = async (row: HTMLElement, item: Row, text: string, tail: RegExp, name: RegExp) => {
  await expectDrawnLine(row)
  const description = await waitFor(() => { const found = line2(row).querySelector<HTMLElement>('[data-mission-eta]'); expect(found).not.toBeNull(); return found! })
  await expect(description).toHaveAttribute('data-mission-eta', text)
  await expect(description.textContent).toMatch(tail)
  await expect(getComputedStyle(description).position).toBe('absolute')
  await expect(rect(description).width).toBeLessThanOrEqual(1)
  await expect(sidebarRowAccessibleName(item)).toMatch(name)
}
/** No ETA: no description, nothing in the name, and the same drawn line. */
const expectNoEta = async (row: HTMLElement, item: Row) => {
  await expectDrawnLine(row)
  await expect(row.querySelector('[data-mission-eta]')).toBeNull()
  await expect(line2(row).textContent).not.toContain('left')
  await expect(sidebarRowAccessibleName(item)).not.toContain('Estimated')
}
// The basis, retained samples, exact as_of and its age relative to the story's fixed now; never a recomputed range.
const etaChecks: Readonly<Record<string, (row: HTMLElement, item: Row) => Promise<void>>> = {
  'ETA range': (row, item) => expectEta(row, item, '~12–18 min left', /^Estimated 12–18 min left, served as of .+, 2m ago\. Critical-path basis, 12 retained samples\.$/,
    /, Step Review hover scopes, Estimated 12–18 min left, served as of 2026-10-21T09:28:00\.000Z\. Critical-path basis, 12 retained samples\.$/),
  'ETA single': (row, item) => expectEta(row, item, '~12 min left', /^Estimated 12 min left, served as of .+, 2m ago\. Per-step basis, 1 retained sample\.$/,
    /, Estimated 12 min left, served as of 2026-10-21T09:28:00\.000Z\. Per-step basis, 1 retained sample\.$/),
  'ETA finishing': (row, item) => expectEta(row, item, 'Finishing', /^No remaining steps, served as of .+, 2m ago\. Critical-path basis\.$/,
    /, No remaining steps, served as of 2026-10-21T09:28:00\.000Z\. Critical-path basis\.$/),
  'ETA null': expectNoEta,
  'ETA absent': expectNoEta,
  'ETA finished': expectNoEta,
}
const etaPlay = (titles: readonly string[]): NonNullable<Story['play']> => async ({ canvasElement }) => {
  await waitFor(() => expect(canvasElement.querySelectorAll('[data-milestone-row]').length).toBe(titles.length))
  for (const title of titles) await etaChecks[title]!(designedRow(canvasElement, title), etaRows.find(item => item.title === title)!)
  // No layout shift between states: every row's drawn line 2 matches the first, to the pixel.
  const [first, ...rest] = titles.map(title => line2Geometry(designedRow(canvasElement, title)))
  for (const geometry of rest) await expect(geometry).toEqual(first)
}
const etaScenario = { scenario: { _tag: 'contract-fixture', reason: 'Fixed served ETA rows prove the exact served description with its basis and as_of, and that no ETA state (range, single, null, absent, finished) changes the drawn row; catalog runs rarely serve every state.' } } as const
const etaState = (title: string, name: string): Story => ({ tags: ['critical'], name: `Mission ETA · ${name}`, parameters: etaScenario, render: ({ scheme }) => <EtaStatesStory scheme={scheme} only={title} />, play: etaPlay([title]) })

export const EtaFinishing: Story = etaState('ETA finishing', 'no remaining steps')
export const EtaRange: Story = etaState('ETA range', 'served range')
export const EtaSingle: Story = etaState('ETA single', 'served single value')
export const EtaNull: Story = etaState('ETA null', 'served null')
export const EtaAbsent: Story = etaState('ETA absent', 'absent')
/** All states side by side, including a finished run whose served ETA must stay hidden. */
export const EtaStates: Story = { tags: ['critical'], name: 'Mission ETA · all states', parameters: etaScenario, render: ({ scheme }) => <EtaStatesStory scheme={scheme} />, play: etaPlay(etaRows.map(item => item.title)) }
export const EtaStatesLight: Story = { ...EtaStates, name: 'Mission ETA · all states · light', args: { scheme: 'light' } }

/** Bounding hit area after every rectangular CSS clip, including a visually hidden ancestor's 1px wrapper. */
const clippedTouchRect = (control: HTMLElement): DOMRect => {
  let box = rect(control)
  const intersect = (left: number, top: number, right: number, bottom: number) => {
    const x = Math.max(box.left, left), y = Math.max(box.top, top)
    box = new DOMRect(x, y, Math.max(0, Math.min(box.right, right) - x), Math.max(0, Math.min(box.bottom, bottom) - y))
  }
  const length = (value: string, size: number, auto: number): number => {
    if (value === 'auto') return auto
    if (!/^-?(?:\d+\.?\d*|\.\d+)(?:px|%)?$/.test(value)) throw new Error(`Unsupported touch-target clip length: ${value}`)
    return parseFloat(value) * (value.endsWith('%') ? size / 100 : 1)
  }
  for (let ancestor: HTMLElement | null = control; ancestor !== null; ancestor = ancestor.parentElement) {
    const style = getComputedStyle(ancestor), bounds = rect(ancestor)
    if (style.position === 'fixed' || style.transform !== 'none') throw new Error('Unsupported touch-target geometry')
    if (style.visibility === 'hidden' || style.display === 'none') return new DOMRect()
    if (ancestor !== control) {
      const clipX = /^(hidden|clip|auto|scroll)$/.test(style.overflowX)
      const clipY = /^(hidden|clip|auto|scroll)$/.test(style.overflowY)
      if (clipX || clipY) {
        const left = bounds.left + ancestor.clientLeft, top = bounds.top + ancestor.clientTop
        intersect(clipX ? left : box.left, clipY ? top : box.top, clipX ? left + ancestor.clientWidth : box.right, clipY ? top + ancestor.clientHeight : box.bottom)
      }
    }
    if (style.clip !== 'auto' && style.clip !== '') {
      const match = /^rect\((.+)\)$/.exec(style.clip)
      if (match === null) throw new Error(`Unsupported touch-target clip: ${style.clip}`)
      const edges = match[1]!.trim().split(/,\s*|\s+/)
      if (edges.length !== 4) throw new Error(`Invalid touch-target clip: ${style.clip}`)
      intersect(bounds.left + length(edges[3]!, bounds.width, 0), bounds.top + length(edges[0]!, bounds.height, 0),
        bounds.left + length(edges[1]!, bounds.width, bounds.width), bounds.top + length(edges[2]!, bounds.height, bounds.height))
    }
    if (style.clipPath !== 'none' && style.clipPath !== '') {
      const match = /^inset\((.+)\)$/.exec(style.clipPath)
      if (match === null) throw new Error(`Unsupported touch-target clip-path: ${style.clipPath}`)
      const values = match[1]!.split(/\s+round\s+/)[0]!.trim().split(/\s+/)
      if (values.length < 1 || values.length > 4) throw new Error(`Invalid touch-target clip-path: ${style.clipPath}`)
      const [top, right = top!, bottom = top!, left = right!] = values
      intersect(bounds.left + length(left!, bounds.width, 0), bounds.top + length(top!, bounds.height, 0),
        bounds.right - length(right!, bounds.width, 0), bounds.bottom - length(bottom!, bounds.height, 0))
    }
  }
  return box
}

/**
 * Touch-layout mission targets in `scope`: each is one button at least 44 × 44, its own box (no pseudo-element), inside
 * its own row, ending before every trailing control (disclosure, actions, any Open action) and clear of the next row.
 * The visible ring inside stays 12px, centred.
 */
const assertTouchTargets = async (scope: HTMLElement) => {
  const rows = [...scope.querySelectorAll<HTMLElement>('[data-milestone-row]')].filter(element => element.closest('[inert]') === null)
  for (const [index, element] of rows.entries()) {
    const trigger = element.querySelector<HTMLElement>('button[data-mission-trigger]')
    await expect(element.hasAttribute('data-touch-mission')).toBe(trigger !== null)
    if (trigger === null) continue
    const box = rect(trigger)
    const own = rect(element)
    await expect(box.width).toBeGreaterThanOrEqual(44)
    await expect(box.height).toBeGreaterThanOrEqual(44)
    await expect(box.left).toBeGreaterThanOrEqual(own.left - 0.5)
    await expect(box.right).toBeLessThanOrEqual(own.right + 0.5)
    await expect(box.top).toBeGreaterThanOrEqual(own.top - 0.5)
    await expect(box.bottom).toBeLessThanOrEqual(own.bottom + 0.5)
    await expect(getComputedStyle(trigger, '::before').content).toMatch(/^(none|normal)$/)
    const inner = rect(trigger.querySelector('[data-mission-ring], [data-queued-count]')!)
    await expect(Math.abs((inner.top + inner.bottom) / 2 - (box.top + box.bottom) / 2)).toBeLessThanOrEqual(1)
    for (const other of element.querySelectorAll<HTMLElement>('button, [data-row-action], [data-agent-menu], [data-sidebar-disclosure]')) {
      const otherBox = clippedTouchRect(other)
      // A clipped-away control is not a competing touch target, regardless of its unclipped DOM rect.
      if (other === trigger || trigger.contains(other) || otherBox.width <= 1 || otherBox.height <= 1) continue
      await expect(intersects(box, otherBox)).toBe(false)
      if (otherBox.left >= box.left) await expect(box.right).toBeLessThanOrEqual(otherBox.left + 0.5)
    }
    const next = rows[index + 1]
    if (next !== undefined) await expect(intersects(box, rect(next))).toBe(false)
  }
}

/** Touch layout at the viewport's width (up to 440px): ring and "+N" merge into one 44px target that opens from a touch press. */
const phonePlay: NonNullable<Story['play']> = async context => {
  const frame = within(context.canvasElement).getByTestId('mission-rows')
  await expect(Math.abs(rect(frame).width - Math.min(document.documentElement.clientWidth, 440))).toBeLessThanOrEqual(1)
  await missionRowsPlay(context)
  const designedHost = context.canvasElement.querySelector<HTMLElement>('[data-host="designed"]')!
  // Exactly the queued and retained rows carry a merged target; plain live rows keep the bare 48px row.
  await expect([...designedHost.querySelectorAll('[data-touch-mission] [data-row-column="title"]')].map(title => title.textContent))
    .toEqual(['Port the upstream sidebar', 'Rename the users API', 'Watch the upload retries', 'Index the search logs', 'Map the billing events'])
  await expect(designedHost.querySelector('[data-queued-missions]:not([data-mission-trigger])')).toBeNull()
  await assertTouchTargets(designedHost)
  // Negative control: an actual overlapping 44px target must still fail the same shared assertion.
  const overlapRow = designedHost.querySelector<HTMLElement>('[data-touch-mission]')!
  const missionTarget = overlapRow.querySelector<HTMLElement>('button[data-mission-trigger]')!
  const overlapping = document.createElement('button')
  overlapping.type = 'button'
  overlapping.textContent = 'Overlapping target'
  const targetBox = rect(missionTarget), rowBox = rect(overlapRow)
  Object.assign(overlapping.style, { position: 'absolute', left: `${targetBox.left - rowBox.left}px`, top: `${targetBox.top - rowBox.top}px`, width: '44px', height: '44px' })
  overlapRow.append(overlapping)
  try {
    const visibleOverlap = clippedTouchRect(overlapping)
    await expect(visibleOverlap.width).toBe(44)
    await expect(visibleOverlap.height).toBe(44)
    await expect(intersects(rect(missionTarget), visibleOverlap)).toBe(true)
    await expect(assertTouchTargets(designedHost)).rejects.toThrow(/expected true to be false/)
  } finally {
    overlapping.remove()
  }
  // Fixed positioning escapes clips; transformed ancestor rects mix viewport and layout coordinates. Fail closed.
  for (const geometry of ['fixed control', 'fixed ancestor', 'transformed ancestor'] as const) {
    const outer = document.createElement('div'), clipped = document.createElement('div'), control = document.createElement('button')
    Object.assign(outer.style, { position: geometry === 'fixed ancestor' ? 'fixed' : 'absolute', left: '0px', top: '0px', transform: geometry === 'transformed ancestor' ? 'scale(2)' : 'none' })
    Object.assign(clipped.style, { position: 'relative', overflow: 'hidden', width: '1px', height: '1px' })
    control.type = 'button'
    control.textContent = geometry
    Object.assign(control.style, { position: geometry === 'fixed control' ? 'fixed' : 'absolute', left: '0px', top: '0px', width: '44px', height: '44px' })
    clipped.append(control)
    outer.append(clipped)
    overlapRow.append(outer)
    try {
      await expect(assertTouchTargets(designedHost)).rejects.toThrow('Unsupported touch-target geometry')
    } finally {
      outer.remove()
    }
  }

  const parentSection = context.canvasElement.querySelector<HTMLElement>('[data-host="mission-parent"]')!
  const parent = parentSection.querySelector<HTMLElement>('[data-milestone-row]')!
  const disclosure = within(parent).getByRole('button', { name: 'Hide subagents' })
  const actions = within(parent).getByRole('button', { name: `Actions for ${missionParent.title}` })
  for (const control of [disclosure, actions]) {
    const box = rect(control)
    await expect(box.width).toBeGreaterThanOrEqual(44)
    await expect(box.height).toBeGreaterThanOrEqual(44)
    await expect(rect(line2(parent)).right).toBeLessThanOrEqual(box.left + 0.5)
  }
  // The parent carries a mission and a queue, so its merged target sits beside both trailing 44px tracks.
  await assertTouchTargets(parentSection)
  const parentTrigger = within(parent).getByRole('button', { name: missionName('+1 more mission queued') })
  await expect(parentTrigger).toHaveAttribute('data-mission-trigger', 'queued')
  await expect(parent.querySelector('[data-line2-slot="step"]')).toHaveTextContent('Write the port notes')
  await expect(rect(parent.querySelector('[data-line2-slot="step"]')!).width).toBeGreaterThan(0)
  // Tapping it opens only the popover: the tree row's action never fires.
  await userEvent.pointer([{ keys: '[TouchA>]', target: parentTrigger }, { keys: '[/TouchA]', target: parentTrigger }])
  const parentDialog = await waitFor(() => { const element = document.querySelector<HTMLElement>('[role="dialog"][aria-label="Queued mission"]'); expect(element).not.toBeNull(); expect(element).toBeVisible(); return element! })
  await expect([...parentDialog.querySelectorAll('[data-queued-mission]')].map(item => item.getAttribute('data-queued-mission'))).toEqual(['sidebar-polish'])
  await expect(parentSection).not.toHaveAttribute('data-opened')
  await waitFor(() => expect(parentDialog).toHaveFocus())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(document.querySelector('[role="dialog"][aria-label="Queued mission"]')).toBeNull())

  const child = parentSection.querySelectorAll<HTMLElement>('[data-milestone-row]')[1]!
  await expect(line2(child).textContent).toBe('Needs you·build-1')
  await expect(child.querySelector('[data-agent-menu], [data-row-action], [data-mission], [data-queued-mission], button')).toBeNull()
  await userEvent.click(child)
  await expect(parentSection).not.toHaveAttribute('data-opened')
  await userEvent.click(disclosure)
  await expect(within(parent).getByRole('button', { name: 'Show subagents' })).toBeVisible()
  await userEvent.click(within(parent).getByRole('button', { name: 'Show subagents' }))
  await expect(within(parentSection).getByText(missionChild.title)).toBeVisible()

  // A touch press on a Failed row's merged target opens "Mission info" with the warning and its queue in one popover.
  const failed = designedRow(context.canvasElement, 'Map the billing events').querySelector<HTMLElement>('button[data-mission-trigger="info"]')!
  await userEvent.pointer([{ keys: '[TouchA>]', target: failed }, { keys: '[/TouchA]', target: failed }])
  const failedDialog = await waitFor(() => { const found = infoDialog(); expect(found).not.toBeNull(); expect(found).toBeVisible(); return found! })
  await expect([...failedDialog.querySelectorAll('[data-queued-mission]')].map(item => item.getAttribute('data-queued-mission'))).toEqual(['docs-refresh'])
  await waitFor(() => expect(failedDialog).toHaveFocus())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(infoDialog()).toBeNull())

  // A touch press opens the queue inside the viewport, not the simulated phone's clipping frame.
  const trigger = within(designedRow(context.canvasElement, 'Port the upstream sidebar')).getByRole('button', { name: missionName('+2 more missions queued') })
  await expect(trigger.querySelector('[data-queued-count]')).toHaveTextContent('+2')
  await userEvent.pointer([{ keys: '[TouchA>]', target: trigger }, { keys: '[/TouchA]', target: trigger }])
  const dialog = await waitFor(() => { const element = queuedDialog(); expect(element).not.toBeNull(); expect(element).toBeVisible(); return element! })
  await expect([...dialog.querySelectorAll('[data-queued-mission]')].map(item => item.getAttribute('data-queued-mission'))).toEqual(['docs-refresh', 'rename-users-api'])
  await waitFor(() => {
    const box = rect(dialog)
    expect(box.left).toBeGreaterThanOrEqual(0)
    expect(box.right).toBeLessThanOrEqual(window.innerWidth)
  })
  await waitFor(() => expect(dialog).toHaveFocus())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(queuedDialog()).toBeNull())

  // A pinned parent whose real row is virtualized away gains a queue in place (its touch row grows to the 44px mission
  // track). The pin takes the content's measured height, never a cached or fixed one, so nothing is clipped.
  // Ends queued and pinned for the a11y check.
  const stickySection = context.canvasElement.querySelector<HTMLElement>('[data-host="sticky-parent"]')!
  const stickyGrid = within(stickySection).getByRole('treegrid')
  stickyGrid.dispatchEvent(new WheelEvent('wheel', { bubbles: true }))
  stickyGrid.scrollTop = stickyGrid.scrollHeight
  const pin = () => stickySection.querySelector<HTMLElement>(`[data-pinned-key="${stickyParent.ref}"]`)
  const pinnedRow = () => pin()?.querySelector<HTMLElement>('[data-milestone-row]') ?? null
  const realParent = () => stickySection.querySelector(`[data-sidebar-tree-key="${stickyParent.ref}"]`)
  const before = await waitFor(() => {
    const row = pinnedRow()
    expect(row).not.toBeNull()
    expect(realParent()).toBeNull()
    expect(row).not.toHaveAttribute('data-touch-mission')
    expect(Math.abs(rect(pin()!).height - rect(row!).height)).toBeLessThanOrEqual(0.5)
    return rect(row!).height
  })
  await userEvent.click(within(stickySection).getByRole('button', { name: 'Queue a mission' }))
  await waitFor(() => {
    const shell = pin()!
    const row = pinnedRow()!
    expect(realParent()).toBeNull()
    expect(row).toHaveAttribute('data-touch-mission')
    expect(rect(row).height).toBeGreaterThan(before + 0.5)
    expect(Math.abs(rect(shell).height - rect(row).height)).toBeLessThanOrEqual(0.5)
    expect(rect(row).bottom).toBeLessThanOrEqual(rect(shell).bottom + 0.5)
    expect(shell.scrollHeight).toBeLessThanOrEqual(shell.clientHeight)
  })
}

export const Phone: Story = { tags: ['critical'], name: 'Phone · merged mission touch target', parameters: { scenario: { _tag: 'contract-fixture', reason: 'A fixed row matrix in a touch-layout frame at the viewport width (390 or 440px) proves pill alignment and truncation, one 44px pill-and-"+N" target per queued or retained-info row kept inside its row and clear of the parent’s disclosure and actions tracks and the next row, Stale/Failed mission info, display-only rows without mission controls, and a pinned parent whose offscreen row gains a queue and grows its pin to the new height unclipped.' } }, args: { scheme: 'dark', phone: true }, play: phonePlay }
export const PhoneLight: Story = { ...Phone, name: 'Phone · merged mission touch target · light', args: { scheme: 'light', phone: true } }


const styles = stylex.create({
  root: { display: 'flex', alignItems: 'flex-start', flexWrap: 'wrap', gap: s.xl, minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  // The live default sidebar: `sidebarDefault` wide, rows inset by `s.md` with 2px gaps.
  column: { display: 'flex', flexDirection: 'column', gap: s.xs2, width: g.sidebarDefault, boxSizing: 'border-box', padding: s.md, backgroundColor: surface.sidebar },
  folder: { display: 'flex', flexDirection: 'column', gap: s.xs2 },
  // The real viewport width up to a large phone: 390px on a 390px viewport, 440px on 440px and wider.
  phoneFrame: { flexDirection: 'column', width: '100%', maxWidth: '440px', padding: 0, gap: 0 },
  phoneColumn: { width: '100%' },
  parentViewport: { height: '144px', minHeight: 0 },
  // The pinned band holds at most 40% of the tree, so the tree must exceed 68 / 0.4 for a queued parent to pin whole.
  stickyViewport: { height: '240px', minHeight: 0 },
  queueToggle: { alignSelf: 'flex-start', minHeight: g.touchTarget },
  heading: { margin: 0, paddingInline: s.sm, paddingBlock: s.xs, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
  // Resolves the surface's own tokens for the plays: attention amber (decision handle) and soft ink (line text).
  toneProbe: { position: 'absolute', width: s.zero, height: s.zero, overflow: 'hidden', color: tone.attention, backgroundColor: ink.fgSoft },
})
