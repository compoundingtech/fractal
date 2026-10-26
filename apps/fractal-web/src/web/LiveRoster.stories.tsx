import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import * as stylex from '@stylexjs/stylex'
import { decodeResourceRows, type DecodedAgent } from '@fractal/sdk/effect'
import oldLiteral from '../../../../packages/fractal-sdk/src/effect/agentSnapshot.oldLiteral.json' with { type: 'json' }
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { lightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { fleetFromAgents } from '../data/projections.ts'
import { observed, type Feed, type Fleet, type RosterDecode } from '../data/source.ts'
import { RosterDecodeNotice, rosterDecodeNotice, shownRosterRefs } from './LiveRoster.tsx'
import { RosterStaleNotice } from './SidebarEmptyState.tsx'

/** Synthetic old-literal snapshot; every other row is a single-field variant of it. */
const wire = oldLiteral.items[0]!
const futureUsageRef = 'agent/example/future-usage'
const brokenRuntimeRef = 'agent/example/broken-runtime'
/** A daemon newer than this view: an aggregation mode the generated codec does not know. */
const futureUsage = { ...wire, id: futureUsageRef, name: 'Future usage agent', usage: { ...wire.usage, aggregation: 'future-aggregation-mode' } }
/** Required structure the view cannot read; the identity stays typed in the diagnostic. */
const brokenRuntime = { ...wire, id: brokenRuntimeRef, name: 'Broken runtime agent', runtime_ids: 'runtime/not-a-list' }
/** No readable identity at all. */
const brokenIdentity = { ...wire, id: 42, name: 'Broken identity agent' }

const decodeAgents = (rows: readonly unknown[]) => {
  const { items, undecodable } = decodeResourceRows(rows)
  return { agents: items.filter((row): row is DecodedAgent => row.kind === 'agent'), undecodable }
}

const partial = decodeAgents([wire, futureUsage, brokenRuntime, brokenIdentity])
const lastGood = decodeAgents([wire, futureUsage])
const allBad = decodeAgents([{ ...wire, runtime_ids: 'runtime/not-a-list' }, brokenRuntime, brokenIdentity])

interface Scenario {
  readonly fleet: Feed<Fleet>
  readonly decode: RosterDecode
  readonly stale: boolean
}
const scenarios = {
  decoded: (() => {
    const { agents } = decodeAgents([wire])
    return { fleet: observed({ value: fleetFromAgents(agents) }), decode: { undecodable: [], decodeStale: false }, stale: false }
  })(),
  partial: {
    fleet: observed({ value: fleetFromAgents(partial.agents) }),
    decode: { undecodable: partial.undecodable, decodeStale: false },
    stale: false,
  },
  // The SDK retains last-good rows when every row of the latest frame fails; the feed turns stale.
  allBad: {
    fleet: observed({ value: fleetFromAgents(lastGood.agents), freshness: 'stale' }),
    decode: { undecodable: allBad.undecodable, decodeStale: true },
    stale: true,
  },
} satisfies Record<string, Scenario>

const Roster = ({ scenario, scheme }: { readonly scenario: keyof typeof scenarios; readonly scheme: 'light' | 'dark' }) => {
  const { fleet, decode, stale } = scenarios[scenario]
  return (
    <aside aria-label="Agents" data-scheme={scheme} {...stylex.props(styles.sidebar, ...(scheme === 'light' ? lightTheme : []))}>
      {fleet._tag === 'Observed' ? <>
        {stale ? <RosterStaleNotice decodeStale={decode.decodeStale} observedAt={undefined} partial={fleet.coverage?._tag === 'Partial'} refreshFailed={false} onRetry={undefined} /> : null}
        <RosterDecodeNotice decode={decode} shown={shownRosterRefs(fleet)} />
      </> : null}
    </aside>
  )
}

const styles = stylex.create({
  sidebar: { width: 280, minHeight: '100vh', display: 'flex', flexDirection: 'column', backgroundColor: c.sidebar, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
})

const meta = {
  title: 'wf/Sidebar/Roster decode',
  component: Roster,
  parameters: { layout: 'fullscreen' },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] } },
  args: { scheme: 'dark' },
} satisfies Meta<typeof Roster>
export default meta
type Story = StoryObj<typeof meta>

const agentRefs = (scenario: keyof typeof scenarios) => {
  const { fleet } = scenarios[scenario]
  if (fleet._tag !== 'Observed') throw new Error('roster must be observed')
  return fleet.value.agents.map((agent) => agent.ref)
}

/** Negative control: the synthetic old-literal row decodes through the SDK and raises no notice. */
export const FullyDecoded: Story = {
  args: { scenario: 'decoded' },
  play: async ({ canvasElement }) => {
    expect(decodeResourceRows([wire]).undecodable).toEqual([])
    expect(agentRefs('decoded')).toEqual([wire.id])
    expect(canvasElement.querySelector('[data-wf-roster-decode]')).toBeNull()
    expect(canvasElement.querySelector('[data-wf-roster-stale]')).toBeNull()
    expect(within(canvasElement).queryByRole('status')).toBeNull()
  },
}

export const UndecodableRow: Story = {
  args: { scenario: 'partial' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Good rows remain, including the row whose usage aggregation is newer than this view.
    expect(agentRefs('partial')).toEqual([wire.id, futureUsageRef])
    const fleet = scenarios.partial.fleet
    if (fleet._tag !== 'Observed') throw new Error('partial roster must be observed')
    expect(fleet.value.agents.find((agent) => agent.ref === wire.id)?.usage._tag).toBe('Known')
    expect(fleet.value.agents.find((agent) => agent.ref === futureUsageRef)?.usage).toEqual({ _tag: 'Unknown' })
    expect(fleet.value.agents.find((agent) => agent.ref === futureUsageRef)?.name).toBe('Future usage agent')

    // One quiet, polite notice; never an alert per failed row.
    const notices = canvasElement.querySelectorAll<HTMLElement>('[data-wf-roster-decode]')
    expect(notices).toHaveLength(1)
    expect(notices[0]!.dataset.wfRosterDecode).toBe('partial')
    const status = canvas.getByRole('status')
    expect(status.textContent).toBe(rosterDecodeNotice(2))
    expect(status).toHaveAttribute('aria-live', 'polite')
    expect(status).toHaveAttribute('aria-atomic', 'true')
    expect(status.textContent).toBe('2 agents could not be shown · this view is older than the daemon')
    expect(canvas.queryByRole('alert')).toBeNull()
    expect(canvasElement.querySelector('[data-wf-roster-stale]')).toBeNull()

    const details = canvas.getByRole('button', { name: 'Details' })
    expect(details).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(details)
    expect(details).toHaveAttribute('aria-expanded', 'true')
    const list = canvas.getByRole('list', { name: 'Agents that could not be shown' })
    expect(list.textContent).toContain(brokenRuntimeRef)
    expect(list).toHaveAttribute('tabindex', '0')
    expect(list.querySelector('[data-wf-decode-path="runtime_ids"]')).not.toBeNull()
    expect(list.querySelector('[data-wf-decode-path="id"]')?.textContent).toContain('Row without a readable id')
    // Shown rows are not listed as hidden.
    expect(list.textContent).not.toContain(futureUsageRef)
    expect(list.textContent).not.toContain(wire.id)
  },
}

export const AllBadStale: Story = {
  args: { scenario: 'allBad' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(allBad.agents).toEqual([])
    // The retained last-good roster stays and is marked stale.
    expect(agentRefs('allBad')).toEqual([wire.id, futureUsageRef])
    const staleMarker = canvasElement.querySelector<HTMLElement>('[data-wf-roster-stale]')
    expect(staleMarker?.dataset.wfRosterStale).toBe('decode')
    expect(staleMarker?.textContent).toBe('Last observed list')

    const notices = canvasElement.querySelectorAll<HTMLElement>('[data-wf-roster-decode]')
    expect(notices).toHaveLength(1)
    expect(notices[0]!.dataset.wfRosterDecode).toBe('stale')
    expect(canvas.getByText('3 agents could not be shown · this view is older than the daemon')).toHaveAttribute('role', 'status')
    expect(canvas.queryByRole('alert')).toBeNull()

    await userEvent.click(canvas.getByRole('button', { name: 'Details' }))
    const list = canvas.getByRole('list', { name: 'Agents that could not be shown' })
    // Every row of the stale frame is listed, including the identity still shown from last-good.
    expect(list.textContent).toContain(wire.id)
    expect(list.textContent).toContain(brokenRuntimeRef)
    expect(list.querySelector('[data-wf-decode-path="id"]')).not.toBeNull()
  },
}
