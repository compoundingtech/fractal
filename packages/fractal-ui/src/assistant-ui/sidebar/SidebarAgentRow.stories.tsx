/** Baseline row coverage: the visible title is the exact accessible name and the complete reported facts the exact description, the custom hover card is the only hover disclosure (no native titles), known-only hover card facts, omitted unreported fields, an aligned known-vs-none time cohort, and the row hover quick Open action. */
import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { AgentHoverCard, SidebarAgentRow, sidebarRowDescription, type RowVariant } from './SidebarAgentRow'
import type { SidebarAgentRow as Row } from './model'
import { accentVars as accent, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t, geometryVars as g, borderVars as border, radiusVars as r, elevationVars as elevation } from '../composition-tokens.stylex'
import { baselineTheme } from '../neutral-theme'
import { lightTheme, type Scheme } from '../composition-theme'
import { ThemePortal } from '../taste/ThemePortal'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

const storyNow = Date.UTC(2026, 9, 8, 12, 0, 0)
/** Fully reported synthetic identity; every optional fact the card can render is present. */
const knownRow: Row = {
  ref: 'agent/sample', id: 'sample', title: 'Sample session', description: 'Reviews the selection model',
  status: 'working', statusLabel: 'Working', host: 'host-1', harness: 'seat-1', model: 'glm-5',
  statusSince: storyNow - 90_000, lastActivityAt: storyNow - 30_000, needsMe: true, unread: 2, freshness: 'live',
  usage: { _tag: 'Known', scope: '24h-root-and-subagents', usd: 1.84, tokens: 412_000 },
  duration: { _tag: 'Known', scope: '24h-activity-span', ms: 3_600_000 },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 300_000 },
  branch: 'feat/rows', worktree: '~/rows',
  pullRequest: { ref: 'agent/sample/pr', title: 'Align row cohort', number: 42, state: 'open' },
  children: [],
}
/** Same identity, nothing reported beyond title, status, host and counts; every optional fact is omitted. */
const unreportedRow: Row = { ...knownRow, ref: 'agent/unreported', id: 'unreported', title: 'Unreported session', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, description: undefined, model: undefined, harness: undefined, pullRequest: undefined, branch: undefined, worktree: undefined, terminal: undefined, mission: undefined, statusSince: undefined, lastActivityAt: undefined }
/** Synthetic cohort: every second row carries no last-turn fact, so its time slot renders empty while the shared metric track keeps every title aligned. */
const cohortRows: readonly Row[] = [
  { ...knownRow, ref: 'agent/first', id: 'first', title: 'First session' },
  { ...knownRow, ref: 'agent/second', id: 'second', title: 'Second session', lastTurn: { _tag: 'Unknown' }, needsMe: false, unread: 0 },
  { ...knownRow, ref: 'agent/third', id: 'third', title: 'Third session', status: 'waiting', statusLabel: 'Needs you' },
  { ...knownRow, ref: 'agent/fourth', id: 'fourth', title: 'Fourth session', lastTurn: { _tag: 'Unknown' }, usage: { _tag: 'Unknown' } },
  { ...unreportedRow, ref: 'agent/fifth', id: 'fifth', title: 'Fifth session' },
  { ...knownRow, ref: 'agent/sample/subagent/child', id: 'sample', parentRef: 'agent/sample', title: 'Check output', lastTurn: { _tag: 'Unknown' }, unread: undefined, description: undefined },
]
/** Synthetic parent with two subagents, for collapsed and expanded child rows. */
const parentRow: Row = { ...knownRow, ref: 'agent/parent', id: 'parent', title: 'Parent session', needsMe: false, children: [
  { ...knownRow, ref: 'agent/parent/subagent/one', id: 'parent', parentRef: 'agent/parent', title: 'Subagent one', needsMe: undefined, children: [] },
  { ...unreportedRow, ref: 'agent/parent/subagent/two', id: 'parent', parentRef: 'agent/parent', title: 'Subagent two', children: [] },
] }
/** Phone fixtures: a title too long for the narrowed title track, and a desktop copy of the parent for the dense control. */
const longRow: Row = { ...cohortRows[2]!, ref: 'agent/long', id: 'long', title: 'Long session title that has to truncate before the touch columns' }
const desktopParentRow: Row = { ...parentRow, ref: 'agent/parent/desktop' }

function CardPair({ row }: { readonly row: Row }) {
  return <div {...stylex.props(styles.spread)}>
    <div data-frame-rows {...stylex.props(styles.list)}><SidebarAgentRow item={row} now={storyNow} active /></div>
    <div {...stylex.props(styles.aside)}>
      <div data-testid="hover-facts"><AgentHoverCard row={row} now={storyNow} /></div>
    </div>
  </div>
}

/** Hover intent opens after 150ms and closes after 300ms; a saturated strict host can stretch that past waitFor's 1s default. */
const hoverSettleTimeout = 5_000
const fixtureByRef = new Map<string, Row>([knownRow, unreportedRow, ...cohortRows, ...cohortRows.map(row => ({ ...row, ref: `${row.ref}/sr1` })), ...cohortRows.map(row => ({ ...row, ref: `${row.ref}/sr3` })), ...parentRow.children, parentRow, longRow, desktopParentRow].map(row => [row.ref, row]))

/** The visible title is the exact name; the complete reported facts are the exact description, carried by a mounted hidden span rather than a native title or the unsupported aria-description. */
async function assertRowNames(canvasElement: HTMLElement) {
  const rows = within(canvasElement).getAllByTestId('taste-agent-row')
  await expect(rows.length).toBeGreaterThan(0)
  for (const row of rows) {
    const fixture = fixtureByRef.get(row.parentElement!.dataset.wfAgentRef!)!
    await expect(fixture).toBeDefined()
    const button = row.querySelector<HTMLButtonElement>(':scope > button')!
    const title = row.querySelector('[data-row-column="title-text"]')!.textContent!
    await expect(title).toBe(fixture.title)
    await expect(button).toHaveAccessibleName(fixture.title)
    // Label-in-name: the button's own visible text is exactly its name; subtitle facts are painted siblings and live in the description.
    await expect(button.textContent).toBe(fixture.title)
    await expect(button).toHaveAccessibleDescription(sidebarRowDescription(fixture))
    await expect(button).toHaveAccessibleDescription(/Status: /)
    if (fixture.needsMe === true) await expect(button).toHaveAccessibleDescription(/Needs your attention/)
    const described = document.getElementById(button.getAttribute('aria-describedby')!)
    await expect(described).not.toBeNull()
    await expect(row.contains(described)).toBe(true)
    await expect(described!.textContent).toBe(sidebarRowDescription(fixture))
    await expect(row.parentElement!.querySelectorAll('[title]')).toHaveLength(0)
    await expect(row.parentElement!.querySelectorAll('[aria-description]')).toHaveLength(0)
  }
}

/** Shared helpers keep each former native title string, exact to the millisecond and ISO instant, as the accessible description of their semantic element. */
async function assertHelperDescriptions(canvasElement: HTMLElement) {
  const wrapper = canvasElement.querySelector<HTMLElement>(`[data-wf-agent-ref="${knownRow.ref}"]`)!
  await expect(within(wrapper).getAllByRole('img')[0]).toHaveAccessibleDescription(`Working. Observed status since ${new Date(knownRow.statusSince!).toISOString()}. Observation live.`)
  await expect(wrapper.querySelector('[data-row-field="total-duration"] time')).toHaveAccessibleDescription('24h activity span: 3600000 milliseconds')
  const lastTurn = knownRow.lastTurn._tag === 'Known' ? knownRow.lastTurn.at : 0
  await expect(wrapper.querySelector('[data-row-field="last-turn"] time')).toHaveAccessibleDescription(`Last completed turn ${new Date(lastTurn).toISOString()}`)
  await expect(wrapper.querySelectorAll('time[title], [role="img"][title]')).toHaveLength(0)
}

/** The custom hover card is the only hover disclosure: hovering any row field opens exactly one card and no native title exists on the field or its ancestors. */
async function assertSingleHoverDisclosure(canvasElement: HTMLElement) {
  const fields = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="taste-agent-row"] :is([data-row-column], [data-row-field], [data-row-signal])')]
  await expect(fields.length).toBeGreaterThan(0)
  for (const field of fields) {
    for (let node: HTMLElement | null = field; node !== null; node = node.parentElement) await expect(node.hasAttribute('title')).toBe(false)
  }
  const row = canvasElement.querySelector<HTMLElement>('[data-testid="taste-agent-row"]')!
  // Resolve, before any card opens, the element that actually receives the pointer at each painted field (subtitle fields pass input through to the row button).
  const targets = [...row.querySelectorAll<HTMLElement>(':is([data-row-column], [data-row-field], [data-row-signal])')].flatMap(field => {
    const box = field.getBoundingClientRect()
    return box.width === 0 || box.height === 0 ? [] : [document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) as HTMLElement]
  })
  await expect(targets.length).toBeGreaterThan(0)
  for (const target of targets) {
    await expect(row.contains(target)).toBe(true)
    for (let node: HTMLElement | null = target; node !== null; node = node.parentElement) await expect(node.hasAttribute('title')).toBe(false)
    await userEvent.hover(target)
    await waitFor(() => expect(document.querySelectorAll('[role="tooltip"] [data-testid="agent-hover-card"]')).toHaveLength(1), { timeout: hoverSettleTimeout })
  }
  await userEvent.unhover(row)
  await waitFor(() => expect(document.querySelectorAll('[role="tooltip"] [data-testid="agent-hover-card"]')).toHaveLength(0), { timeout: hoverSettleTimeout })
}

/** Card text a sighted user sees: the clone drops every hidden description carrier that an aria-describedby references. */
async function paintedText(card: HTMLElement): Promise<string> {
  const painted = card.cloneNode(true) as HTMLElement
  const carriers = [...card.querySelectorAll('[aria-describedby]')].flatMap(described => described.getAttribute('aria-describedby')!.split(' '))
  await expect(carriers.length).toBeGreaterThan(0)
  for (const id of carriers) painted.querySelector(`#${CSS.escape(id)}`)!.parentElement!.remove()
  return painted.textContent!
}

async function assertCompactCard(canvasElement: HTMLElement) {
  await assertRowNames(canvasElement)
  const cards = within(canvasElement).getAllByTestId('agent-hover-card')
  await expect(cards).toHaveLength(1)
  const card = cards[0]!
  await expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(400)
  const labels = [...card.querySelectorAll('dt')].map(label => label.textContent)
  await expect(new Set(labels).size).toBe(labels.length)
  for (const label of ['Name', 'ID', 'Description', 'Last activity', 'Worktree', 'Lease expires']) await expect(labels).not.toContain(label)
  await expect(card.querySelectorAll('li').length).toBeLessThanOrEqual(3)
  // Exact instants stay in the hidden description carriers only; the painted card text never shows ISO timestamps.
  const painted = await paintedText(card)
  await expect(painted).not.toMatch(/\d{4}-\d{2}-\d{2}T\d{2}:/)
}
function CohortList() {
  return <div data-frame-rows {...stylex.props(styles.list)}>{cohortRows.map((row, index) => <SidebarAgentRow key={row.ref} item={row} now={storyNow} active={index === 0} />)}</div>
}

/** A themed surface whose overlays portal through ThemePortal; the hidden probe resolves the surface's own tokens. Rows are openable, so their buttons take keyboard focus. */
function ThemedOverlays({ scheme }: { readonly scheme: Scheme }) {
  const [openedRef, setOpenedRef] = React.useState(cohortRows[0]!.ref)
  const open = React.useCallback((row: Row) => setOpenedRef(row.ref), [])
  return <main data-testid="themed-overlays" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <span aria-hidden="true" data-token-probe="" {...stylex.props(styles.tokenProbe)} />
    <div data-frame-rows {...stylex.props(styles.list)}>{cohortRows.slice(0, 3).map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} onOpen={open} active={openedRef === row.ref} />)}</div>
  </ThemePortal></main>
}
async function assertThemedOverlay(surfaceElement: HTMLElement, overlay: Element, raised: string) {
  await expect(surfaceElement.contains(overlay)).toBe(true)
  await expect(getComputedStyle(overlay).backgroundColor).toBe(raised)
}
async function assertNeutralRing(element: Element, focus: string) {
  const style = getComputedStyle(element)
  await expect(style.outlineStyle).toBe('solid')
  await expect(style.outlineColor).toBe(focus)
}

/** Every density with one fixture set; SR-1/SR-3 copies carry distinct refs so each row maps back to its fixture. */
function DensityMatrix() {
  const [expanded, setExpanded] = React.useState(false)
  const toggle = React.useCallback(() => setExpanded(value => !value), [])
  const variants: readonly [RowVariant, string][] = [['SR-1', '/sr1'], ['SR-2', ''], ['SR-3', '/sr3']]
  return <div data-frame-rows {...stylex.props(styles.list)}>
    {variants.flatMap(([variant, suffix]) => cohortRows.map(row => <SidebarAgentRow key={`${row.ref}${suffix}`} item={{ ...row, ref: `${row.ref}${suffix}` }} variant={variant} now={storyNow} />))}
    <SidebarAgentRow item={parentRow} now={storyNow} treeExpanded={expanded} onToggleChildren={toggle} />
  </div>
}

function QuickOpenCohort() {
  const [openedRef, setOpenedRef] = React.useState<string | undefined>()
  const open = React.useCallback((row: Row) => setOpenedRef(row.ref), [])
  const select = React.useCallback((id: string) => setOpenedRef(`agent/${id}`), [])
  return <div data-frame-rows {...stylex.props(styles.list)}>{cohortRows.slice(0, 3).map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} actions={{ select }} onOpen={open} active={openedRef === row.ref} />)}</div>
}

/** Phone roster: the host provides `NarrowLayoutContext`, so each row gives its actions menu and subagent disclosure their own trailing 44px columns. The same rows without the context are the desktop control. */
function PhoneRows() {
  const [expanded, setExpanded] = React.useState(false)
  const toggle = React.useCallback(() => setExpanded(value => !value), [])
  const select = React.useCallback(() => {}, [])
  const rows = [cohortRows[0]!, longRow, cohortRows[3]!]
  return <div {...stylex.props(styles.spread)}>
    <NarrowLayoutContext.Provider value={true}>
      <div data-frame-rows data-phone-rows {...stylex.props(styles.list, styles.phoneList)}>
        {rows.map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} actions={{ select }} />)}
        <SidebarAgentRow item={parentRow} now={storyNow} actions={{ select }} treeExpanded={expanded} onToggleChildren={toggle} />
      </div>
    </NarrowLayoutContext.Provider>
    <div data-frame-rows data-desktop-rows {...stylex.props(styles.list)}><SidebarAgentRow item={desktopParentRow} now={storyNow} actions={{ select }} onToggleChildren={toggle} /></div>
  </div>
}

const meta = {
  title: 'Fractal UI/Sidebar/Agent Row',
  component: SidebarAgentRow,
  // label-content-name-mismatch is an experimental axe rule, so the strict a11y check enables it explicitly for every row story.
  parameters: { layout: 'fullscreen', a11y: { config: { rules: [{ id: 'label-content-name-mismatch', enabled: true }] } }, docs: { description: { component: 'Baseline agent row: reported facts only. The hover card and reported details omit unreported fields instead of showing placeholders; hovering a row swaps the time slot for a quick Open action; rows without a reported time keep the shared metric track so titles stay aligned.' } } },
} satisfies Meta
export default meta
type Story = StoryObj
export const HoverCardKnown: Story = { tags: ['critical'], name: 'Hover card · fully reported', render: () => <main {...stylex.props(styles.root)}><CardPair row={knownRow} /></main>, play: async ({ canvasElement }) => assertCompactCard(canvasElement) }
export const HoverCardUnknown: Story = { tags: ['critical'], name: 'Hover card · unreported fields omitted', render: () => <main {...stylex.props(styles.root)}><CardPair row={unreportedRow} /></main>, play: async ({ canvasElement }) => {
  const card = within(canvasElement).getByTestId('hover-facts')
  await assertCompactCard(canvasElement)
  await expect(await paintedText(card)).not.toMatch(/unknown|unavailable|—/i)
  for (const label of ['Spend', 'Duration', 'Last turn', 'Model', 'PR', 'Branch']) await expect(within(card).queryByText(label)).toBeNull()
} }
export const TimeCohort: Story = { tags: ['critical'], name: 'Time cohort · known vs none', render: () => <main {...stylex.props(styles.root)}><CohortList /></main>, play: async ({ canvasElement }) => {
  await assertRowNames(canvasElement)
  const rows = within(canvasElement).getAllByTestId('taste-agent-row')
  const child = rows.find(row => row.textContent?.includes('Check output'))!
  await expect(child).toBeDefined()
  await expect(child.querySelector('[data-row-field="last-turn"]')).toBeNull()
  const titleLeft = rows[0]!.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect().left
  for (const row of rows) await expect(Math.abs(row.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect().left - titleLeft)).toBeLessThanOrEqual(1)
} }
/** Synthetic last-observed pair: same facts, one working and one idle, both from a stale observation. */
const frozenRows: readonly Row[] = [
  { ...unreportedRow, ref: 'agent/frozen-working', id: 'frozen-working', title: 'Frozen working session', status: 'working', statusLabel: 'Last verified · working', freshness: 'stale', needsMe: false, unread: 0 },
  { ...unreportedRow, ref: 'agent/frozen-idle', id: 'frozen-idle', title: 'Frozen idle session', status: 'idle', statusLabel: 'Last verified · idle', freshness: 'stale', needsMe: false, unread: 0 },
  { ...unreportedRow, ref: 'agent/frozen-stale', id: 'frozen-stale', title: 'Frozen stale session', status: 'stale', statusLabel: 'Stale observation', freshness: 'stale', needsMe: false, unread: 0 },
]
export const FrozenWorkingVersusIdle: Story = { tags: ['critical'], name: 'Frozen observation · working vs idle', render: () => <main {...stylex.props(styles.root)}><div {...stylex.props(styles.list)}>{frozenRows.map(row => <SidebarAgentRow key={row.ref} item={row} now={storyNow} />)}</div></main>, play: async ({ canvasElement }) => {
  const glyphs = [...canvasElement.querySelectorAll<HTMLElement>('[data-row-column="status"] [data-status]')]
  await expect(glyphs.map(glyph => glyph.dataset.status)).toEqual(['working', 'idle', 'stale'])
  for (const glyph of glyphs) await expect(glyph).toHaveAttribute('data-frozen', 'true')
  // One muted tone: a frozen observation never reads as live and stale is never the danger red.
  await expect(new Set(glyphs.map(glyph => getComputedStyle(glyph).color)).size).toBe(1)
  // Nothing spins.
  for (const svg of canvasElement.querySelectorAll('[data-row-column="status"] svg')) await expect(getComputedStyle(svg).animationName).toBe('none')
  // Without color, shape tells working from idle: working keeps its static ring, idle is the plain disc.
  const [working, idle] = glyphs.map(glyph => glyph.querySelector('svg')!)
  await expect(working!.querySelectorAll('circle, path').length).toBeGreaterThan(idle!.querySelectorAll('circle, path').length)
} }
export const QuickOpenAction: Story = { tags: ['critical'], name: 'Row hover · quick Open', render: () => <main {...stylex.props(styles.root)}><QuickOpenCohort /></main>, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  const rows = canvas.getAllByTestId('taste-agent-row')
  const target = rows[1]!
  await document.fonts.ready
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  await userEvent.hover(target)
  await userEvent.click(within(target).getByRole('button', { name: /^Open / }))
  await expect(target.querySelector('[aria-current="page"]')).not.toBeNull()
  await assertRowNames(canvasElement)
  // Keyboard focus still opens the custom card and Enter still opens the row.
  await userEvent.unhover(target)
  const third = rows[2]!.querySelector<HTMLButtonElement>(':scope > button')!
  third.focus()
  await waitFor(() => expect(document.querySelectorAll('[role="tooltip"] [data-testid="agent-hover-card"]')).toHaveLength(1), { timeout: hoverSettleTimeout })
  await userEvent.keyboard('{Enter}')
  await waitFor(() => expect(rows[2]!.querySelector('[aria-current="page"]')).not.toBeNull())
  await assertRowNames(canvasElement)
} }
export const AllStates: Story = { tags: ['critical'], render: () => <main {...stylex.props(styles.root)}><CardPair row={knownRow} /><CardPair row={unreportedRow} /><CohortList /></main>, play: async ({ canvasElement }) => {
  await assertRowNames(canvasElement)
  await assertHelperDescriptions(canvasElement)
  await assertSingleHoverDisclosure(canvasElement)
  // Negative control: an injected native title must be rejected, then the passing state is restored.
  const probe = canvasElement.querySelector<HTMLElement>('[data-row-field="host"]')!
  probe.setAttribute('title', 'injected')
  await expect(assertRowNames(canvasElement)).rejects.toThrow()
  probe.removeAttribute('title')
  await assertRowNames(canvasElement)
} }
export const DensitiesAndChildren: Story = { tags: ['critical'], name: 'Names and descriptions · SR-1/2/3, collapsed and expanded', render: () => <main {...stylex.props(styles.root)}><DensityMatrix /></main>, play: async ({ canvasElement }) => {
  await assertRowNames(canvasElement)
  await assertSingleHoverDisclosure(canvasElement)
  const toggle = within(canvasElement).getByRole('button', { name: 'Expand 2 subagents' })
  await userEvent.click(toggle)
  await waitFor(() => expect(within(canvasElement).getByRole('button', { name: 'Collapse 2 subagents' })).toHaveAttribute('aria-expanded', 'true'))
  await expect(within(canvasElement).getByRole('button', { name: 'Subagent two' })).toBeInTheDocument()
  await assertRowNames(canvasElement)
  await userEvent.click(within(canvasElement).getByRole('button', { name: 'Collapse 2 subagents' }))
  await waitFor(() => expect(within(canvasElement).queryByRole('button', { name: 'Subagent two' })).toBeNull())
  await assertRowNames(canvasElement)
  // The pointer rests on the row, so its hover card opens; the a11y check must see it open, not race its delay.
  await waitFor(() => expect(document.querySelectorAll('[role="tooltip"] [data-testid="agent-hover-card"]')).toHaveLength(1), { timeout: hoverSettleTimeout })
} }
export const PhoneTouchTargets: Story = { tags: ['critical'], name: 'Phone · 44px menu and disclosure columns', render: () => <main {...stylex.props(styles.root)}><PhoneRows /></main>, play: async ({ canvasElement }) => {
  await document.fonts.ready
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  const phone = canvasElement.querySelector<HTMLElement>('[data-phone-rows]')!
  const rows = within(phone).getAllByTestId('taste-agent-row')
  await expect(rows).toHaveLength(4)
  for (const row of rows) {
    await expect(row).toHaveAttribute('data-narrow', 'true')
    const frame = row.getBoundingClientRect()
    // SR-2 keeps its 48px height, status glyph and metric slot.
    await expect(Math.abs(frame.height - 48)).toBeLessThanOrEqual(0.5)
    await expect(row.querySelector('[data-row-column="status"]')!.getBoundingClientRect().width).toBeGreaterThan(0)
    await expect(row.querySelector('[data-row-column="time"]')).not.toBeNull()
    const open = row.querySelector<HTMLElement>(':scope > button')!.getBoundingClientRect()
    const title = row.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect()
    const menuButton = row.querySelector<HTMLElement>('[data-agent-menu]')!
    const menu = menuButton.getBoundingClientRect()
    // Touch has no hover: the menu is visible, at least 44×44, inside the row and clear of the open target.
    await expect(getComputedStyle(menuButton).opacity).toBe('1')
    await expect(menu.width).toBeGreaterThanOrEqual(44)
    await expect(menu.height).toBeGreaterThanOrEqual(44)
    await expect(menu.right).toBeLessThanOrEqual(frame.right + 0.5)
    await expect(menu.top).toBeGreaterThanOrEqual(frame.top - 0.5)
    await expect(menu.bottom).toBeLessThanOrEqual(frame.bottom + 0.5)
    await expect(menu.left).toBeGreaterThanOrEqual(open.right - 0.5)
    await expect(title.right).toBeLessThanOrEqual(open.right + 0.5)
    const disclosure = row.querySelector<HTMLElement>('[data-row-column="children"]')
    if (disclosure !== null) {
      const box = disclosure.getBoundingClientRect()
      await expect(box.width).toBeGreaterThanOrEqual(44)
      await expect(box.height).toBeGreaterThanOrEqual(44)
      await expect(box.left).toBeGreaterThanOrEqual(open.right - 0.5)
      await expect(box.right).toBeLessThanOrEqual(menu.left + 0.5)
    }
  }
  // The long title truncates before the touch columns instead of pushing them out.
  const longTitle = rows[1]!.querySelector<HTMLElement>('[data-row-column="title-text"]')!
  await expect(longTitle.scrollWidth).toBeGreaterThan(longTitle.clientWidth)
  // The disclosure expands and collapses from its 44px target; the nested rows keep the phone grid.
  await userEvent.click(within(rows[3]!).getByRole('button', { name: 'Expand 2 subagents' }))
  await waitFor(() => expect(within(phone).getByRole('button', { name: 'Collapse 2 subagents' })).toHaveAttribute('aria-expanded', 'true'))
  for (const child of within(phone).getAllByTestId('taste-agent-row').slice(4)) await expect(child.querySelector<HTMLElement>('[data-agent-menu]')!.getBoundingClientRect().width).toBeGreaterThanOrEqual(44)
  await userEvent.click(within(phone).getByRole('button', { name: 'Collapse 2 subagents' }))
  await waitFor(() => expect(within(phone).queryByRole('button', { name: 'Subagent two' })).toBeNull())
  // A tap on the menu opens 44px items; Escape closes and returns focus to the trigger.
  const trigger = within(rows[0]!).getByRole('button', { name: 'Actions for First session' })
  await userEvent.click(trigger)
  const items = await waitFor(() => { const found = document.querySelectorAll<HTMLElement>('[role="menu"] [role="menuitem"]'); expect(found.length).toBeGreaterThan(0); return found })
  // The standard popover scales during entry; measure the settled touch geometry.
  for (const item of items) await waitFor(() => expect(item.getBoundingClientRect().height).toBeGreaterThanOrEqual(44))
  // A hover card the pointer left behind (the expanded row it just collapsed) closes on its own 300ms delay; the menu opening must leave none.
  await waitFor(() => expect(document.querySelector('[role="tooltip"]')).toBeNull())
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(document.querySelector('[role="menu"]')).toBeNull())
  await waitFor(() => expect(document.activeElement).toBe(trigger))
  await assertRowNames(canvasElement)
  // Desktop control: without the context the menu and disclosure keep their dense geometry.
  const desktop = within(canvasElement.querySelector<HTMLElement>('[data-desktop-rows]')!).getByTestId('taste-agent-row')
  await expect(desktop).not.toHaveAttribute('data-narrow')
  await expect(desktop.querySelector('[data-agent-menu]')!.getBoundingClientRect().width).toBeLessThan(44)
  await expect(desktop.querySelector('[data-row-column="children"]')!.getBoundingClientRect().height).toBeLessThan(44)
} }
export const ThemedOverlaysAndFocus: StoryObj<{ scheme: Scheme }> = {
  name: 'Themed overlays · row menu and neutral focus',
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <ThemedOverlays scheme={scheme} />,
  play: async ({ canvasElement }) => {
    const themed = within(canvasElement).getByTestId('themed-overlays')
    const probe = getComputedStyle(themed.querySelector('[data-token-probe]')!)
    const raised = probe.backgroundColor, focus = probe.color, accentColor = probe.borderTopColor
    await expect(focus).not.toBe(accentColor)
    const row = within(canvasElement).getAllByTestId('taste-agent-row')[0]!
    const gear = row.querySelector<HTMLElement>('[data-agent-menu]')!
    gear.focus()
    await userEvent.keyboard('{Enter}')
    const menu = await waitFor(() => { const element = document.querySelector('[role="menu"]'); if (element === null) throw new Error('menu not open'); return element })
    const popover = menu.closest('[data-trigger="MenuTrigger"]')!
    await assertThemedOverlay(themed, popover, raised)
    const item = await waitFor(() => { const element = menu.querySelector('[role="menuitem"][data-focus-visible]'); if (element === null) throw new Error('no keyboard-focused item'); return element })
    await assertNeutralRing(item, focus)
    await userEvent.keyboard('r')
    const reportedDetails = within(menu as HTMLElement).getByRole('menuitem', { name: 'Reported agent details' })
    await expect(reportedDetails).toHaveFocus()
    await assertNeutralRing(reportedDetails, focus)
    // Negative controls: an overlay outside the themed surface and an accent ring must both be rejected.
    const escaped = document.createElement('div')
    escaped.className = popover.className
    document.body.append(escaped)
    await expect(assertThemedOverlay(themed, escaped, raised)).rejects.toThrow()
    escaped.remove()
    ;(item as HTMLElement).style.outlineColor = accentColor
    await expect(assertNeutralRing(item, focus)).rejects.toThrow()
    ;(item as HTMLElement).style.removeProperty('outline-color')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(document.querySelector('[role="menu"]')).toBeNull())
    // Keyboard focus on the active row: the same neutral ring.
    const rowButton = row.querySelector<HTMLElement>(':scope > button')!
    rowButton.focus()
    await userEvent.keyboard('{Shift}')
    await waitFor(() => expect(rowButton).toHaveAttribute('data-focus-visible'))
    await assertNeutralRing(rowButton, focus)
  },
}

const styles = stylex.create({
  root: { height: '100vh', minHeight: 0, overflowY: 'auto', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, display: 'flex', flexDirection: 'column', gap: s.xl },
  spread: { display: 'flex', alignItems: 'flex-start', flexWrap: 'wrap', gap: s.xl },
  list: { display: 'flex', flexDirection: 'column', gap: s.xs, width: g.specimenAside, minWidth: 0 },
  phoneList: { width: 360, maxWidth: '100%' },
  aside: { width: g.specimenAside, maxWidth: '100%', minWidth: 0, boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, boxShadow: elevation.popover },
  tokenProbe: { position: 'absolute', width: 0, height: 0, overflow: 'hidden', backgroundColor: surface.raised, color: ink.fgMuted, borderTopStyle: 'solid', borderTopWidth: 0, borderTopColor: accent.primary },
})
