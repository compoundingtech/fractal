/**
 * The canonical entity hover card: one primitive, one card grammar, one card per entity wherever it appears. The
 * sidebar row, a workspace tab, a transcript mention and a resource row all ask an `EntityCardSource` for the same ref
 * and so show the identical card. Plays prove identity, timing, keyboard focus, dismissal, the null veto and that only
 * one card is open at a time. `TasteVariants` lays out the open design questions side by side.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button, Dialog, DialogTrigger, Link, Modal } from 'react-aria-components'
import { AgentHoverCard } from '../sidebar/DeferredRowActions.tsx'
import type { SidebarAgentRow as Row, SidebarOtherMission } from '../sidebar/model'
import { borderVars as border, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { EntityCardSource, EntityHoverCard, type EntityCardResolver } from './EntityHoverCard'
import { EntityCard, EntityCardHeader, EntityCardText } from './EntityCard'
import { HoverCard } from './HoverCard'
import { MissionEntityCard } from './MissionCard'
import { FolderEntityCard, ResourceEntityCard } from './ResourceCard'
import { agentDescriptor } from './agent'
import { hoverCardTiming, type EntityRef } from './entity.ts'

const storyNow = Date.UTC(2026, 10, 7, 9, 30, 0)
const mission: SidebarOtherMission = { id: 'mission/sidebar-polish', slug: 'sidebar-polish', title: 'Polish the sidebar rows', progress: { _tag: 'Known', done: 3, total: 7 }, step: { _tag: 'Step', name: 'Review hover scopes' } }
const agent: Row = {
  ref: 'agent/polish', id: 'polish', title: 'Polish the sidebar rows', status: 'working', statusLabel: 'Working', host: 'build-2', freshness: 'live',
  description: 'Unify hover cards across entity types',
  usage: { _tag: 'Known', scope: 'lifetime-incl-subagents', usd: 3.42, tokens: 1_180_000 }, duration: { _tag: 'Known', scope: 'lifetime', ms: 2_460_000 },
  lastTurn: { _tag: 'Known', kind: 'turn-completed', at: storyNow - 95_000 }, model: 'opus', branch: 'schickling-assistant/hover-cards',
  missionContext: { _tag: 'Mission', ...mission, queued: [{ id: 'mission/tabs', slug: 'tabs', title: 'Workspace tabs' }] },
  children: [],
}
const agentRef: EntityRef = { kind: 'agent', id: agent.ref }
const missionRef: EntityRef = { kind: 'mission', id: mission.id }
const prRef: EntityRef = { kind: 'resource', id: 'github:compoundingtech/fractal/pull/5741' }
const fileRef: EntityRef = { kind: 'resource', id: 'file:packages/fractal-ui/src/assistant-ui/entity/HoverCard.tsx' }
const folderRef: EntityRef = { kind: 'folder', id: 'folder/web' }

const pullRequest = <ResourceEntityCard id={prRef.id} title="Unify entity hover cards" kind="Pull request" icon="pull-request" status={{ tone: 'working', label: 'Open' }}
  summary={['compoundingtech/fractal', '#5741']} text="One hover-card primitive and per-entity adapters for agents, missions, resources and folders."
  meta={[{ icon: 'person', label: 'Author', value: 'schickling' }, { icon: 'check', label: 'Checks', value: '12 of 14 passed' }, { icon: 'clock', label: 'Updated', value: '4 minutes ago' }]} />
const file = <ResourceEntityCard id={fileRef.id} title="HoverCard.tsx" kind="File" icon="attach" summary={['packages/fractal-ui/src/assistant-ui/entity', '+142 −0']}
  preview={['+export function HoverCard({ children, content }) {', '+  const intent = useHoverCardIntent()', '+  return <>…</>', '+}']} previewKind="diff" />

/** The host's lookup: the same feeds the sidebar row reads, keyed by entity ref. */
const resolve: EntityCardResolver = entity => {
  if (entity.kind === 'agent' && entity.id === agent.ref) return <AgentHoverCard row={agent} reportedStatusOnly now={storyNow} />
  if (entity.kind === 'mission' && entity.id === mission.id) return <MissionEntityCard mission={mission} agents={[agent.title]} queued={['Workspace tabs']} now={storyNow} />
  if (entity.kind === 'resource' && entity.id === prRef.id) return pullRequest
  if (entity.kind === 'resource' && entity.id === fileRef.id) return file
  if (entity.kind === 'folder' && entity.id === folderRef.id) return <FolderEntityCard id={folderRef.id} name="Web app" host="juniper" agents={[agent.title, 'Workspace tabs', 'Header panel']} needsYou={1} />
  return undefined
}

const cards = () => Array.from(document.querySelectorAll<HTMLElement>('[data-hover-card]'))
const cardFor = async (entity: EntityRef) => waitFor(() => {
  const open = cards()
  expect(open).toHaveLength(1)
  const card = open[0]!.querySelector<HTMLElement>(`[data-entity-card="${entity.kind}"][data-entity-id="${entity.id}"]`)
  expect(card).not.toBeNull()
  return card!
})
const settleClosed = () => waitFor(() => expect(cards()).toHaveLength(0), { timeout: hoverCardTiming.closeMs + 1000 })
/** Polls past the open delay: no card may appear at any point in the window. */
const staysShut = async (ms: number) => {
  const started = performance.now()
  do {
    await expect(cards()).toHaveLength(0)
    await new Promise(resolve => setTimeout(resolve, 20))
  } while (performance.now() - started < ms)
}

/** Four surfaces that show one agent: a sidebar row, a workspace tab, a transcript mention and a resource row. */
function Surfaces() {
  return <EntityCardSource resolve={resolve}>
    <div {...stylex.props(styles.page)}>
      <section aria-label="Sidebar" {...stylex.props(styles.panel)}>
        <EntityHoverCard entity={agentRef} placement="right top"><Button data-surface="sidebar-row" {...stylex.props(styles.row)}>{agent.title}</Button></EntityHoverCard>
        <EntityHoverCard entity={folderRef} placement="right top"><Button data-surface="folder-row" {...stylex.props(styles.row)}>Web app</Button></EntityHoverCard>
      </section>
      <section aria-label="Workspace" {...stylex.props(styles.panel)}>
        <div {...stylex.props(styles.tabs)}>
          <EntityHoverCard entity={agentRef} placement="bottom start"><Button data-surface="tab" {...stylex.props(styles.tab)}>{agent.title}</Button></EntityHoverCard>
          <EntityHoverCard entity={missionRef} placement="bottom start"><Button data-surface="mission-tab" {...stylex.props(styles.tab)}>{mission.title}</Button></EntityHoverCard>
        </div>
        <p {...stylex.props(styles.prose)}>
          Handed off to <EntityHoverCard entity={agentRef}><Link data-surface="mention" href="#agent" {...stylex.props(styles.link)}>{agent.title}</Link></EntityHoverCard>, which opened{' '}
          <EntityHoverCard entity={prRef}><Link data-surface="pr-mention" href="#pr" {...stylex.props(styles.link)}>#5741</Link></EntityHoverCard> and touched{' '}
          <EntityHoverCard entity={fileRef}><Link data-surface="file-mention" href="#file" {...stylex.props(styles.link)}>HoverCard.tsx</Link></EntityHoverCard>.
        </p>
      </section>
      <section aria-label="Details" {...stylex.props(styles.panel)}>
        <EntityHoverCard entity={agentRef} placement="left top"><Button data-surface="resource-row" {...stylex.props(styles.row)}>{agent.title}</Button></EntityHoverCard>
      </section>
    </div>
  </EntityCardSource>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Entity hover card',
  component: Surfaces,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'One hover-card primitive and one card grammar for agents, missions, resources and folders. Every surface resolves the card by entity ref, so the same entity shows the identical card in a sidebar row, a tab, a transcript mention and a resource row.' } } },
} satisfies Meta<typeof Surfaces>
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Gallery:contract-fixture',
  'KeyboardFocusOpens:primitive',
  'ModalEscapePriority:primitive',
  'NullContentStaysShut:primitive',
  'OneCardAtATime:contract-fixture',
  'SameEntitySameCard:contract-fixture',
  'TasteVariants:contract-fixture',
]

const fixture = (reason: string) => ({ scenario: { _tag: 'contract-fixture' as const, reason } })

/** A sidebar row, a tab, a transcript mention and a resource row naming one agent each open the identical card. */
export const SameEntitySameCard: Story = {
  tags: ['critical'],
  parameters: fixture('A fixed agent, mission, PR, file and folder resolved by one entity source verify that every surface naming the same agent opens the identical sidebar card.'),
  play: async ({ canvasElement }) => {
    const markup: string[] = []
    for (const surface of ['sidebar-row', 'tab', 'mention', 'resource-row']) {
      const trigger = canvasElement.querySelector<HTMLElement>(`[data-surface="${surface}"]`)!
      await userEvent.hover(trigger)
      const card = await cardFor(agentRef)
      // The sidebar card is the baseline: its proof id, the agent's status, host, work and missions.
      await expect(card).toHaveAttribute('data-testid', 'agent-hover-card')
      await expect(card).toHaveTextContent(agent.title)
      await expect(within(card).getByTestId('hover-missions')).toHaveTextContent('Polish the sidebar rows')
      // Generated ids differ per mount; everything else (structure, text, attributes) must match.
      markup.push(card.outerHTML.replace(/\s(id|aria-describedby|aria-labelledby|aria-controls|for)="[^"]*"/g, ''))
      await userEvent.unhover(trigger)
      await settleClosed()
    }
    await expect(new Set(markup).size).toBe(1)
  },
}

/** Keyboard focus that the user moved opens the card like a resting pointer; Escape closes it and focus stays put. */
export const KeyboardFocusOpens: Story = {
  parameters: { scenario: { _tag: 'primitive', reason: 'The hover-card primitive over a fixed entity source: keyboard focus, Escape and blur, with no domain data.' } },
  play: async ({ canvasElement }) => {
    const trigger = canvasElement.querySelector<HTMLElement>('[data-surface="tab"]')!
    trigger.focus()
    await userEvent.tab({ shift: true })
    await userEvent.tab()
    await expect(trigger).toHaveFocus()
    const card = await cardFor(agentRef)
    // The open card is the trigger's description; closing it removes the reference.
    const surface = card.closest<HTMLElement>('[data-hover-card]')!
    await expect(trigger.getAttribute('aria-describedby')?.split(' ')).toContain(surface.id)
    await userEvent.keyboard('{Escape}')
    await settleClosed()
    await expect(trigger).toHaveFocus()
    await expect(trigger).not.toHaveAttribute('aria-describedby')
    await userEvent.tab()
    await settleClosed()
  },
}

/** A passive preview consumes the first Escape even when the focused textbox belongs to an enclosing modal. */
export const ModalEscapePriority: Story = {
  tags: ['critical'],
  parameters: { scenario: { _tag: 'primitive', reason: 'A dismissible modal with a separate textbox and pointer-opened hover card verifies that the first Escape closes only the passive card and preserves focus, while a second Escape dismisses the modal.' } },
  render: () => <DialogTrigger>
    <Button>Open preview modal</Button>
    <Modal isDismissable><Dialog aria-label="Preview modal">
      <label>Modal composer<input aria-label="Modal composer" /></label>
      <HoverCard content={<p>Reported preview facts</p>}><Button>Preview entity</Button></HoverCard>
    </Dialog></Modal>
  </DialogTrigger>,
  play: async ({ canvasElement }) => {
    const trigger = within(canvasElement).getByRole('button', { name: 'Open preview modal' })
    await userEvent.click(trigger)
    const modal = await within(document.body).findByRole('dialog', { name: 'Preview modal' })
    const composer = within(modal).getByRole('textbox', { name: 'Modal composer' })
    await userEvent.click(composer)
    await userEvent.hover(within(modal).getByRole('button', { name: 'Preview entity' }))
    await waitFor(() => expect(document.querySelector('[data-hover-card]')).not.toBeNull())
    await expect(composer).toHaveFocus()
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(document.querySelector('[data-hover-card]')).toBeNull())
    await expect(modal).toBeVisible()
    await expect(composer).toHaveFocus()
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(within(document.body).queryByRole('dialog', { name: 'Preview modal' })).toBeNull())
    // RAC FocusScope restores focus in requestAnimationFrame after unmount, not when the dialog first disappears.
    await waitFor(() => expect(trigger).toHaveFocus())
  },
}

/** Opening a second card closes the first at once: a pass across rows never stacks cards. */
export const OneCardAtATime: Story = {
  parameters: fixture('Fixed agent and folder rows verify that hovering a second trigger replaces the first card instead of stacking on it.'),
  play: async ({ canvasElement }) => {
    const row = canvasElement.querySelector<HTMLElement>('[data-surface="sidebar-row"]')!
    const folder = canvasElement.querySelector<HTMLElement>('[data-surface="folder-row"]')!
    await userEvent.hover(row)
    await cardFor(agentRef)
    await userEvent.unhover(row)
    await userEvent.hover(folder)
    const card = await cardFor(folderRef)
    await expect(card).toHaveTextContent('1 needs you')
    await userEvent.unhover(folder)
    await settleClosed()
  },
}

/** A source with nothing to show (a plugin without its routes) keeps the card shut: the trigger stays a plain link. */
export const NullContentStaysShut: Story = {
  parameters: { scenario: { _tag: 'primitive', reason: 'The hover-card primitive with a body that returns null: the card never opens, with no domain data.' } },
  render: () => <div {...stylex.props(styles.page)}><HoverCard content={() => null}><Link data-surface="plain" href="#plain" {...stylex.props(styles.link)}>an unknown link</Link></HoverCard></div>,
  play: async ({ canvasElement }) => {
    const trigger = canvasElement.querySelector<HTMLElement>('[data-surface="plain"]')!
    await userEvent.hover(trigger)
    await staysShut(hoverCardTiming.openMs * 3)
    await userEvent.unhover(trigger)
  },
}

/** Every adapter in the shared grammar, side by side, as their cards paint inside the canonical frame. */
export const Gallery: Story = {
  parameters: fixture('Fixed agent, mission, pull request, file and folder data render each adapter in the shared card grammar.'),
  render: () => <div {...stylex.props(styles.gallery)}>
    {[agentRef, missionRef, prRef, fileRef, folderRef].map(entity => <figure key={entity.id} {...stylex.props(styles.figure)}>
      <figcaption {...stylex.props(styles.caption)}>{entity.kind}</figcaption>
      <div {...stylex.props(styles.frame)}>{resolve(entity)}</div>
    </figure>)}
  </div>,
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-entity-card]')).toHaveLength(5))
    for (const kind of ['agent', 'mission', 'resource', 'folder']) await expect(canvasElement.querySelector(`[data-entity-card="${kind}"]`)).not.toBeNull()
  },
}

/** The open design questions, each as two or three variants: compact vs full card on tabs and mentions, passive vs actions. */
export const TasteVariants: Story = {
  parameters: fixture('Fixed agent data render the design variants for the entity hover card: card depth per surface and passive versus interactive cards.'),
  render: () => <div {...stylex.props(styles.variants)}>
    <section {...stylex.props(styles.variant)}>
      <h3 {...stylex.props(styles.variantTitle)}>Q1-A · Full card everywhere (recommended)</h3>
      <p {...stylex.props(styles.variantNote)}>A tab, a mention and a resource row show the sidebar card unchanged. One card to learn, and tab = sidebar exactly as asked.</p>
      <div {...stylex.props(styles.frame)}>{resolve(agentRef)}</div>
    </section>
    <section {...stylex.props(styles.variant)}>
      <h3 {...stylex.props(styles.variantTitle)}>Q1-B · Compact on tabs and mentions</h3>
      <p {...stylex.props(styles.variantNote)}>Header and lead line only; the full card stays on sidebar rows. Lighter over prose, but a tab no longer equals its row.</p>
      <div {...stylex.props(styles.frame)}>
        <EntityCard entity={agentDescriptor(agent)}><EntityCardHeader title={agent.title} summary={[agent.statusLabel, agent.host]} /><EntityCardText>{agent.description}</EntityCardText></EntityCard>
      </div>
    </section>
    <section {...stylex.props(styles.variant)}>
      <h3 {...stylex.props(styles.variantTitle)}>Q2-B · Interactive card with actions</h3>
      <p {...stylex.props(styles.variantNote)}>Open, Split right and Copy link in a footer. The card must take pointer and focus (a non-modal dialog), so it can steal hovers from the rows beneath and needs a longer close grace. Q2-A (recommended) keeps the card passive and actions on the row menu.</p>
      <div {...stylex.props(styles.frame)}>
        {resolve(agentRef)}
        <div {...stylex.props(styles.actions)}><Button {...stylex.props(styles.action)}>Open</Button><Button {...stylex.props(styles.action)}>Split right</Button><Button {...stylex.props(styles.action)}>Copy link</Button></div>
      </div>
    </section>
    <section {...stylex.props(styles.variant)}>
      <h3 {...stylex.props(styles.variantTitle)}>Q3 · Open delay</h3>
      <p {...stylex.props(styles.variantNote)}>A · 150 ms open / 300 ms close, the sidebar today (recommended). B · 350 ms, the old tab and kit ContextCard. C · 500 ms, the tab-bar branch. Longer delays cut flashes while scanning tabs, but make cards feel slow on rows.</p>
    </section>
  </div>,
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-testid="agent-hover-card"]')).toHaveLength(2))
  },
}

const styles = stylex.create({
  page: { display: 'flex', flexWrap: 'wrap', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, minHeight: '100vh', boxSizing: 'border-box', alignItems: 'start' },
  panel: { display: 'flex', flexDirection: 'column', gap: s.sm, width: 280, padding: s.md, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md },
  row: { textAlign: 'start', padding: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit', ':hover': { backgroundColor: surface.rowHover } },
  tabs: { display: 'flex', gap: s.xs },
  tab: { padding: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.rowHover, color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit', maxWidth: 130, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
  prose: { margin: 0, lineHeight: t.bodyLeading, fontSize: t.bodySize },
  link: { color: ink.fg, textDecorationLine: 'underline' },
  gallery: { display: 'flex', flexWrap: 'wrap', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, alignItems: 'start' },
  figure: { margin: 0, display: 'flex', flexDirection: 'column', gap: s.sm },
  caption: { color: ink.fgMuted, fontSize: t.denseSize },
  frame: { backgroundColor: surface.raised, color: ink.fg, borderWidth: 1, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, padding: s.lg, display: 'flex', flexDirection: 'column', gap: s.md },
  variants: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(380px, 1fr))', gap: s.xl, padding: s.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  variant: { display: 'flex', flexDirection: 'column', gap: s.sm },
  variantTitle: { margin: 0, fontSize: t.uiSize },
  variantNote: { margin: 0, color: ink.fgSoft },
  actions: { display: 'flex', gap: s.sm, paddingBlockStart: s.sm, borderTopWidth: 1, borderTopStyle: 'solid', borderTopColor: border.border },
  action: { paddingInline: s.md, paddingBlock: s.xs, borderWidth: 1, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit' },
})
