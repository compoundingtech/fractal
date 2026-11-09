/**
 * The thing kit: every kind drawn from one face as row, fact row, section, inline chip and hover-card body, at the
 * side panel's default (360px) and minimum (280px) widths. Plays prove the decided row rules: title first on one
 * line, meta giving way whole (status word, then spoken id, then age), act labels truncating but never dropping,
 * bare and `seen …` ages, no amber on read-only things, and fact rows that do not react.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor } from 'storybook/test'
import { borderVars as border, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { hoverCardTiming } from '../entity/entity.ts'
import { ThingCard, ThingInline, ThingRow, ThingSection, type ThingSectionItem } from './Thing'
import type { Observed, ThingFace, ThingOpenable } from './face'

const now = Date.UTC(2026, 10, 9, 12, 0, 0)
const ago = (minutes: number) => now - minutes * 60_000
const live = (value: ThingFace, extra: { readonly readOnly?: boolean } = {}): Observed<ThingFace> => ({ _tag: 'Known', value, observedAt: ago(0.2), freshness: 'live', ...extra })
const stale = (value: ThingFace, seenMinutes: number): Observed<ThingFace> => ({ _tag: 'Known', value, observedAt: ago(seenMinutes), freshness: 'stale' })
/** Stories open nothing: the spread proves the affordance (role, focus, hover card) without a workspace. */
const openable: ThingOpenable = { onClick: () => {}, onKeyDown: () => {}, onPointerDown: () => {} }

const faces = {
  agent: { kind: 'Agent', icon: 'message', title: 'Polish the side panel rows', mark: { shape: 'filled', word: 'working' }, at: { at: ago(3), verb: 'last turn' }, summary: 'Reviewing hover scopes on resource rows.' },
  mission: { kind: 'Mission', icon: 'checklist', title: 'Rename the workspace kinds', mark: { shape: 'filled', word: 'running' }, act: { tone: 'act', label: 'pick a naming scheme' }, at: { at: ago(4), verb: 'updated' }, progress: { done: 3, total: 7 } },
  todo: { kind: 'To-do', icon: 'check', title: 'Write the VRS delta for resource renderers', mark: { shape: 'hollow', word: 'pending' } },
  terminal: { kind: 'Terminal', icon: 'terminal', title: 'cargo test -p fractal-github', mark: { shape: 'ended', word: 'exited 101' }, act: { tone: 'failed', label: 'exit 101' }, at: { at: ago(12), verb: 'started' }, summary: 'test result: FAILED. 41 passed; 1 failed', facts: [{ label: 'Handle', value: { _tag: 'Known', value: 'pty 3' }, mono: true }] },
  document: { kind: 'Document', icon: 'attach', title: 'doc/resource-system/interview.md', titleStyle: 'path', mark: { shape: 'hollow', word: 'saved' }, at: { at: ago(26), verb: 'edited' } },
  pullRequest: { kind: 'Pull request', icon: 'pull-request', title: 'Draw panel rows from one face per kind', id: '#5741', mark: { shape: 'hollow', word: 'open' }, act: { tone: 'act', label: 'changes requested' }, at: { at: ago(4), verb: 'updated' }, facts: [{ label: 'Repository', value: { _tag: 'Known', value: 'compoundingtech/fractal' }, mono: true }, { label: 'Checks', value: { _tag: 'Known', value: '12 of 14 passed' } }] },
  issue: { kind: 'Issue', icon: 'dot', title: 'Hover cards stack when rows scroll', id: '#5702', mark: { shape: 'hollow', word: 'open' }, at: { at: ago(95), verb: 'updated' } },
  ciRun: { kind: 'CI run', icon: 'clock', title: 'fractal-web checks', id: '#88112', mark: { shape: 'filled', word: 'running' }, at: { at: ago(6), verb: 'started' } },
  ciFailed: { kind: 'CI run', icon: 'check', title: 'fractal-web checks', id: '#88097', mark: { shape: 'ended', word: 'failed' }, act: { tone: 'failed', label: 'checks failing' }, at: { at: ago(40), verb: 'finished' } },
  review: { kind: 'Review', icon: 'person', title: 'Review of #5741 by mira', mark: { shape: 'ended', word: 'submitted' }, act: { tone: 'act', label: 'changes requested' }, at: { at: ago(9), verb: 'submitted' } },
  resource: { kind: 'Resource', icon: 'info', title: 'atlas staging deploy', mark: { shape: 'none', word: '' }, at: { at: ago(180), verb: 'seen' } },
  subagent: { kind: 'Subagent', icon: 'person', title: 'Survey every caller of DetailsResources', mark: { shape: 'filled', word: 'running' }, at: { at: ago(2), verb: 'started' }, facts: [{ label: 'Type', value: { _tag: 'Known', value: 'explore' }, mono: true }] },
  worktree: { kind: 'Worktree', icon: 'folder', title: 'worktrees/resource-s1', titleStyle: 'path', mark: { shape: 'none', word: 'declared' } },
  binding: { kind: 'Declared binding', icon: 'sliders', title: 'github', mark: { shape: 'none', word: 'declared' } },
} satisfies Record<string, ThingFace>

/** Openable kinds have a durable ref; facts (subagent, worktree, declared binding) have none yet. */
const kindRows: readonly { readonly key: string; readonly observed: Observed<ThingFace>; readonly fact?: true }[] = [
  { key: 'agent', observed: live(faces.agent) },
  { key: 'mission', observed: live(faces.mission) },
  { key: 'todo', observed: live(faces.todo), fact: true },
  { key: 'terminal', observed: live(faces.terminal) },
  { key: 'document', observed: live(faces.document) },
  { key: 'pull-request', observed: live(faces.pullRequest) },
  { key: 'issue', observed: live(faces.issue) },
  { key: 'ci-run', observed: live(faces.ciRun) },
  { key: 'review', observed: live(faces.review) },
  { key: 'resource', observed: live(faces.resource) },
  { key: 'subagent', observed: live(faces.subagent), fact: true },
  { key: 'worktree', observed: live(faces.worktree), fact: true },
  { key: 'binding', observed: live(faces.binding), fact: true },
]

const stateRows: readonly { readonly key: string; readonly observed: Observed<ThingFace>; readonly fact?: true }[] = [
  { key: 'loading', observed: { _tag: 'Loading', hint: { kind: 'Pull request', icon: 'pull-request' } } },
  { key: 'loading-titled', observed: { _tag: 'Loading', hint: { kind: 'Mission', icon: 'checklist', title: 'Rename the workspace kinds' } } },
  { key: 'unavailable', observed: { _tag: 'Unavailable', hint: { kind: 'Terminal', icon: 'terminal', title: 'harbor-2' }, reason: 'This terminal is on a machine that is offline.' } },
  { key: 'invalid', observed: { _tag: 'Invalid', hint: { kind: 'CI run', icon: 'clock' }, issue: 'missing run number' } },
  { key: 'stale', observed: stale(faces.mission, 25) },
  { key: 'read-only-ask', observed: live(faces.pullRequest, { readOnly: true }) },
  { key: 'read-only-failed', observed: live(faces.ciFailed, { readOnly: true }) },
  { key: 'act', observed: live(faces.review) },
  { key: 'failed', observed: live(faces.ciFailed) },
  { key: 'long-act', observed: live({ ...faces.mission, act: { tone: 'act', label: 'choose between the three naming schemes in the proposal' } }) },
  { key: 'fact', observed: live(faces.subagent), fact: true },
  { key: 'fact-act', observed: live({ ...faces.subagent, mark: { shape: 'ended', word: 'failed' }, act: { tone: 'failed', label: 'tool call timed out after 10 minutes' } }), fact: true },
]

const widths = [360, 280] as const

function Column({ width, children }: { readonly width: number; readonly children: React.ReactNode }) {
  return <section aria-label={`${width}px panel`} data-width={width} {...stylex.props(styles.column)} style={{ width }}>
    <h2 {...stylex.props(styles.caption)}>{width}px</h2>
    {children}
  </section>
}

function RowList({ rows }: { readonly rows: typeof kindRows }) {
  return <div role="list">
    {rows.map(row => <div role="listitem" key={row.key} data-row-key={row.key}>
      <ThingRow observed={row.observed} now={now} {...(row.fact === true ? {} : { openable, hoverCard: () => <ThingCard observed={row.observed} now={now} /> })} />
    </div>)}
  </div>
}

const sectionItems = (rows: typeof kindRows): readonly ThingSectionItem[] =>
  rows.map(row => ({ key: row.key, observed: row.observed, ...(row.fact === true ? {} : { openable, hoverCard: () => <ThingCard observed={row.observed} now={now} /> }) }))

const githubSection = sectionItems([
  { key: 'issue', observed: live(faces.issue) },
  { key: 'ci-run', observed: live(faces.ciRun) },
  { key: 'pr-merged', observed: live({ ...faces.pullRequest, id: '#5690', title: 'Quiet the conversation header', mark: { shape: 'ended', word: 'merged' }, act: undefined, at: { at: ago(300), verb: 'updated' } } as ThingFace) },
  { key: 'pull-request', observed: live(faces.pullRequest) },
  { key: 'ci-failed', observed: live(faces.ciFailed) },
  { key: 'pr-old', observed: live({ ...faces.pullRequest, id: '#5655', title: 'Name every subject kind', mark: { shape: 'ended', word: 'merged' }, act: undefined, at: { at: ago(1500), verb: 'updated' } } as ThingFace) },
])

/* ---------- plays ---------- */

const rowOf = (root: HTMLElement, width: number, key: string) => root.querySelector<HTMLElement>(`[data-width="${width}"] [data-row-key="${key}"] [data-thing-row]`)!
/** Meta items on the row's one painted line; wrapped items sit on the clipped second line. */
const shownMeta = (row: HTMLElement): string[] => {
  const meta = row.querySelector<HTMLElement>('[data-thing-meta]')
  if (meta === null) return []
  const bottom = meta.getBoundingClientRect().bottom
  return Array.from(meta.querySelectorAll<HTMLElement>('[data-thing-meta-item]'))
    .filter(item => item.getBoundingClientRect().top < bottom - 1)
    .map(item => item.dataset.thingMetaItem!)
}
const cards = () => Array.from(document.querySelectorAll<HTMLElement>('[data-hover-card]'))

/* ---------- meta ---------- */

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Things',
  parameters: { layout: 'fullscreen', docs: { description: { component: 'One face per kind; the kit draws every row, fact row, section and chip from it, at the side panel’s 360px default and 280px minimum.' } } },
} satisfies Meta
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Chips:contract-fixture',
  'MetaGivesWayInOrder:contract-fixture',
  'RowsByKind:contract-fixture',
  'Sections:contract-fixture',
  'States:contract-fixture',
]

const fixture = (reason: string) => ({ scenario: { _tag: 'contract-fixture' as const, reason } })

/** Every kind as a row at 360px and 280px: openable kinds react and carry a hover card; facts draw the same anatomy. */
export const RowsByKind: Story = {
  tags: ['critical'],
  parameters: fixture('Invented faces for every S1 kind (agent, mission, to-do, terminal, document, PR, issue, CI run, review, generic resource, subagent, worktree, declared binding) at both panel widths.'),
  render: () => <div {...stylex.props(styles.page)}>{widths.map(width => <Column key={width} width={width}><RowList rows={kindRows} /></Column>)}</div>,
  play: async ({ canvasElement }) => {
    for (const width of widths) {
      for (const row of kindRows) {
        const element = rowOf(canvasElement, width, row.key)
        // One line at every width: the row never grows past its single-line height.
        await expect(element.getBoundingClientRect().height).toBeLessThanOrEqual(32)
        if (row.fact === true) {
          await expect(element).not.toHaveAttribute('role')
          await expect(element).not.toHaveAttribute('tabindex')
          await expect(element.querySelector('[tabindex], button, a')).toBeNull()
          await expect(getComputedStyle(element).cursor).not.toBe('pointer')
        } else {
          await expect(element).toHaveAttribute('role', 'link')
          await expect(element).toHaveAttribute('tabindex', '0')
        }
      }
      // Spoken ids only on PR, issue and CI rows; the terminal's pty handle stays in its card.
      const ids = kindRows.filter(row => rowOf(canvasElement, width, row.key).querySelector('[data-thing-meta-item="id"]') !== null).map(row => row.key)
      await expect(ids).toEqual(['pull-request', 'issue', 'ci-run'])
    }
    // An openable row's hover card opens on rest; a fact row's never does.
    const pr = rowOf(canvasElement, 360, 'pull-request')
    await userEvent.hover(pr)
    await waitFor(() => expect(cards()).toHaveLength(1))
    await expect(cards()[0]).toHaveTextContent('updated 4m')
    await userEvent.unhover(pr)
    await waitFor(() => expect(cards()).toHaveLength(0), { timeout: hoverCardTiming.closeMs + 1000 })
    const fact = rowOf(canvasElement, 360, 'subagent')
    await userEvent.hover(fact)
    const rest = Promise.withResolvers<void>()
    setTimeout(rest.resolve, hoverCardTiming.openMs * 3)
    await rest.promise
    await expect(cards()).toHaveLength(0)
    await userEvent.unhover(fact)
  },
}

/** Narrowing one row: the status word goes first, then the spoken id, then the age, each whole; the act stays. */
export const MetaGivesWayInOrder: Story = {
  parameters: fixture('One invented pull request without an act and one with an act, narrowed step by step from 360px to 160px.'),
  render: () => <div {...stylex.props(styles.page)}>
    {[360, 320, 280, 240, 200, 160].map(width => <Column key={width} width={width}><RowList rows={[
      // A short title, so the default width shows every meta item and narrowing shows the order they give way in.
      { key: 'plain', observed: live({ ...faces.pullRequest, title: 'Fix panel rows', act: undefined } as ThingFace) },
      { key: 'act', observed: live(faces.pullRequest) },
    ]} /></Column>)}
  </div>,
  play: async ({ canvasElement }) => {
    const order = ['age', 'id', 'status']
    const counts: number[] = []
    for (const width of [360, 320, 280, 240, 200, 160]) {
      const shown = shownMeta(rowOf(canvasElement, width, 'plain'))
      // Whatever is shown is a prefix of age · id · status: status drops first, then id, then age.
      await expect(shown).toEqual(order.slice(0, shown.length))
      counts.push(shown.length)
      // The act label never drops: it keeps painted width at every step.
      const act = rowOf(canvasElement, width, 'act').querySelector<HTMLElement>('[data-thing-act]')!
      await expect(act.getBoundingClientRect().width).toBeGreaterThan(0)
      await expect(act).toHaveTextContent('changes requested')
    }
    await expect(counts).toEqual([...counts].sort((a, b) => b - a))
    await expect(counts[0]).toBe(3)
  },
}

/** Loading, unavailable, unreadable, stale, read-only, act, failed, a long act and fact rows, at both widths. */
export const States: Story = {
  tags: ['critical'],
  parameters: fixture('Invented faces in every observed state at both panel widths.'),
  render: () => <div {...stylex.props(styles.page)}>{widths.map(width => <Column key={width} width={width}><RowList rows={stateRows} /></Column>)}</div>,
  play: async ({ canvasElement }) => {
    for (const width of widths) {
      await expect(rowOf(canvasElement, width, 'loading')).toHaveAttribute('aria-busy', 'true')
      await expect(rowOf(canvasElement, width, 'loading').querySelector('[data-thing-skeleton]')).not.toBeNull()
      await expect(rowOf(canvasElement, width, 'unavailable')).toHaveAttribute('data-thing-row', 'Unavailable')
      // Stale keeps its amber and says when it was seen.
      const staleRow = rowOf(canvasElement, width, 'stale')
      await expect(staleRow.querySelector('[data-thing-act]')).toHaveAttribute('data-thing-act', 'act')
      await expect(staleRow.querySelector('[data-thing-meta-item="age"]')).toHaveTextContent('seen 25m')
      // Read-only: the ask has no amber; a failure stays red.
      const quiet = rowOf(canvasElement, width, 'read-only-ask').querySelector<HTMLElement>('[data-thing-act]')!
      await expect(quiet).toHaveAttribute('data-thing-act', 'quiet')
      const amber = rowOf(canvasElement, width, 'act').querySelector<HTMLElement>('[data-thing-act]')!
      await expect(getComputedStyle(quiet).color).not.toBe(getComputedStyle(amber).color)
      await expect(rowOf(canvasElement, width, 'read-only-failed').querySelector('[data-thing-act]')).toHaveAttribute('data-thing-act', 'failed')
      // A long act truncates inside the row instead of dropping or wrapping.
      const long = rowOf(canvasElement, width, 'long-act')
      const longAct = long.querySelector<HTMLElement>('[data-thing-act]')!
      await expect(longAct.scrollWidth).toBeGreaterThan(longAct.clientWidth)
      await expect(long.getBoundingClientRect().height).toBeLessThanOrEqual(32)
      // Bare ages: no verb on rows.
      await expect(rowOf(canvasElement, width, 'failed').querySelector('[data-thing-meta-item="age"]')).toHaveTextContent(/^40m$/)
      for (const key of ['fact', 'fact-act']) await expect(rowOf(canvasElement, width, key)).not.toHaveAttribute('role')
    }
  },
}

/** Sections: urgent rows first, then active, then newest; `Show N more`; an unreported kind said once; empty kinds hidden. */
export const Sections: Story = {
  parameters: fixture('Invented GitHub, subagent and review sections at both panel widths, with a whole-feed reconnect line.'),
  render: () => <div {...stylex.props(styles.page)}>{widths.map(width => <Column key={width} width={width}>
    <ThingSection title={`Pull requests · issues · CI — ${width}px`} items={githubSection} now={now} cap={5} />
    <ThingSection title={`Subagents — ${width}px`} items={sectionItems([{ key: 'subagent', observed: live(faces.subagent), fact: true }, { key: 'subagent-done', observed: live({ ...faces.subagent, title: 'Map the registry callers', mark: { shape: 'ended', word: 'completed' }, at: { at: ago(30), verb: 'started' } } as ThingFace), fact: true }])} now={now} notice="Reconnecting; rows show when each was last seen." />
    <ThingSection title={`Reviews — ${width}px`} items={[]} now={now} unknown="This session doesn’t report reviews." />
    <ThingSection title="Declared bindings" items={[]} now={now} />
  </Column>)}</div>,
  play: async ({ canvasElement }) => {
    const column = canvasElement.querySelector<HTMLElement>('[data-width="360"]')!
    const github = column.querySelector<HTMLElement>('[data-thing-section="Pull requests · issues · CI — 360px"]')!
    const titles = Array.from(github.querySelectorAll<HTMLElement>('[data-thing-title]')).map(title => title.textContent)
    await expect(titles).toEqual([faces.pullRequest.title, faces.ciFailed.title, faces.ciRun.title, faces.issue.title, 'Quiet the conversation header'])
    await userEvent.click(Array.from(github.querySelectorAll('button')).find(button => button.textContent === 'Show 1 more')!)
    await expect(github.querySelectorAll('[data-thing-row]')).toHaveLength(6)
    await expect(column.querySelector('[data-thing-section="Reviews — 360px"] [data-thing-section-unknown]')).toHaveTextContent('doesn’t report reviews')
    await expect(column.querySelector('[data-thing-section="Declared bindings"]')).toBeNull()
  },
}

/** Inline chips: bare refs show title, spoken id and live mark; written link text keeps its words behind icon and dot. */
export const Chips: Story = {
  parameters: fixture('Invented PR, CI, mission and document faces as bare and written chips in a sentence, plus loading and read-only chips.'),
  render: () => <div {...stylex.props(styles.page)}>{widths.map(width => <Column key={width} width={width}>
    <p {...stylex.props(styles.prose)}>
      Opened <ThingInline observed={live(faces.pullRequest)} now={now} openable={openable} hoverCard={() => <ThingCard observed={live(faces.pullRequest)} now={now} />} /> after
      {' '}<ThingInline observed={live(faces.ciFailed)} now={now} openable={openable} text="the last run" /> failed; see
      {' '}<ThingInline observed={live(faces.document)} now={now} openable={openable} /> and
      {' '}<ThingInline observed={live(faces.mission)} now={now} openable={openable} />.
      Shared: <ThingInline observed={live(faces.pullRequest, { readOnly: true })} now={now} openable={openable} />,
      {' '}<ThingInline observed={{ _tag: 'Loading', hint: { kind: 'Issue', icon: 'dot', title: '#5702' } }} now={now} openable={openable} />.
    </p>
  </Column>)}</div>,
  play: async ({ canvasElement }) => {
    const column = canvasElement.querySelector<HTMLElement>('[data-width="360"]')!
    const chips = Array.from(column.querySelectorAll<HTMLElement>('[data-thing-inline]'))
    await expect(chips[0]).toHaveTextContent(`${faces.pullRequest.title}#5741`)
    await expect(chips[0]!.querySelector('[data-thing-dot="act"]')).not.toBeNull()
    await expect(chips[1]).toHaveAttribute('data-thing-inline', 'written')
    await expect(chips[1]).toHaveTextContent(/^the last run$/)
    await expect(chips[1]!.querySelector('[data-thing-dot="failed"]')).not.toBeNull()
    await expect(chips[2]).toHaveTextContent('interview.md')
    // Read-only: no amber dot, the neutral mark instead.
    await expect(chips[4]!.querySelector('[data-thing-dot]')).toBeNull()
  },
}

const styles = stylex.create({
  page: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl, padding: s.xl, minHeight: '100vh', boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  column: { display: 'flex', flexDirection: 'column', gap: s.xs, flexShrink: 0, boxSizing: 'border-box', padding: s.sm, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.sidebar },
  caption: { margin: 0, paddingInline: s.sm, color: ink.fgMuted, fontSize: t.metaSize, fontWeight: t.weightRegular },
  prose: { margin: 0, paddingInline: s.sm, fontSize: t.uiSize, lineHeight: t.bodyLeading, color: ink.fgSoft },
})
