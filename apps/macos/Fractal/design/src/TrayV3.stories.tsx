/**
 * Attention tray V3, as history: what #9344 shipped before the native round replaced it with V4
 * (AttentionTray/V4). Ada picked L1 A3 H3 S2 M3 from the V2 exploration (AttentionTray/V2);
 * with K1–K13 they were locked. The images are the frozen V3 renders; its views are gone.
 */
import type { Meta, StoryObj } from '@storybook/react'

import { useAppearance } from './appearance.tsx'
import { MenuBarItem, menuBarStateIds } from './MenuBarItem.tsx'
import { RenderFigure } from './RenderedPanel.tsx'
import { appearances, menuBarStates, resolveV3, v3History, type Appearance } from './renders.ts'

/** Ada' V2 picks (2026-11-13), now the only design. */
const picks: readonly { readonly id: string; readonly text: string }[] = [
  {
    id: 'L1',
    text: 'Inbox first: asks, updates and speech fill the panel; availability and objectives are chips below the inbox that expand their section in place, one at a time. Objectives that need attention start open.',
  },
  {
    id: 'A3',
    text: 'Each ask is a compact row (kind or urgency symbol, title, asker, age, deadline) with a trailing disclosure that opens its detail and inline answers. The first ask and the ask whose answer leads (K4) start open, and so does any ask the person has started answering; a committed ask stays closed.',
  },
  {
    id: 'H3',
    text: 'Minimal header: title, freshness and refresh. The version, Settings… and Quit sit in the footer.',
  },
  {
    id: 'S2',
    text: 'Objectives and budgets are compact rows; a disclosure opens the full 24-hour histogram with hour and percent axes. In each section the worst row that is not met starts open.',
  },
  {
    id: 'M3',
    text: 'Status item: one template symbol plus the open-ask count as plain text, like the battery percentage.',
  },
]

/** Decisions carried over from V2 (K1–K12) and added since (K13), restated for the locked design. */
const locked: readonly { readonly id: string; readonly text: string }[] = [
  {
    id: 'K1',
    text: 'No panel background: the menu-bar window’s Liquid Glass is the surface; content sits on it in grouped Form sections (no glass on glass, no popovers from the panel).',
  },
  {
    id: 'K2',
    text: 'Text styles only; section headers are subheadline semibold secondary under the one headline title; footers are secondary metadata.',
  },
  {
    id: 'K3',
    text: 'One leading edge: title, chips and footer share the Form’s content inset (30 pt); disclosure chevrons trail.',
  },
  {
    id: 'K4',
    text: 'At most one prominent button: the first answerable ask’s recommended answer (never a decline), while nothing is being typed. No answer takes Return: answers are irreversible and the next ask moves into place after a commit.',
  },
  {
    id: 'K5',
    text: 'No hover-only actions: Mark Read, Snooze, Dismiss, Open in Fractal, Read Aloud and an open ask’s answers are always in place; keyboard and VoiceOver reach every control and every disclosure.',
  },
  {
    id: 'K6',
    text: '“Nothing to Do” is an answer button beside the other answers; Read Aloud is icon-only and becomes “Stop Speaking” while that ask is read.',
  },
  {
    id: 'K7',
    text: 'Urgency is a symbol plus weight (monochrome urgent bubble, clock badge + semibold deadline), never orange text alone.',
  },
  {
    id: 'K8',
    text: 'Speech is a grouped section in the inbox: now speaking with Stop and Mute this agent, the queue with Skip, the call toggle, mutes.',
  },
  {
    id: 'K9',
    text: 'Typed answers and the note draft live on the model and survive the panel closing.',
  },
  {
    id: 'K10',
    text: 'Settings… (⌘,) holds Show in Menu Bar; opening the app again brings a hidden item back.',
  },
  {
    id: 'K11',
    text: 'Offline keeps the last count and swaps the status symbol for the sync-problem symbol; objectives stay in the panel (no gauge glyph).',
  },
  {
    id: 'K12',
    text: 'Histograms: Swift Charts bars per hour; an hour without a bar carries a baseline mark that differs in shape, not only color (spent: red cross, unmeasured: grey ring); the nearly-spent zone is a dashed rule; a key under the chart names the rule and the marks.',
  },
  {
    id: 'K13',
    text: 'Snooze (a preset menu) and Dismiss are borderless icon-only controls in the ask row’s link line and beside Open in Fractal in an open ask; updates: Snooze beside Mark Read. Their outcome is a status row in words, never color alone; snoozed cards leave the list and a “N snoozed · first back …” line says so. Seen has no control: a card shown in the open panel is marked seen.',
  },
]

const Decisions = ({
  title,
  decisions,
}: {
  title: string
  decisions: readonly { readonly id: string; readonly text: string }[]
}) => (
  <section className="at-gallery-row" aria-label={title}>
    <h2 className="at-gallery-heading">{title}</h2>
    <ul className="at-sources">
      {decisions.map((decision) => (
        <li key={decision.id}>
          <strong>{decision.id}</strong> {decision.text}
        </li>
      ))}
    </ul>
  </section>
)

const stateIds = menuBarStates.length > 0 ? menuBarStates.map((state) => state.id) : menuBarStateIds

/** One frozen V3 scenario. */
const V3Panel = ({ scenario, appearance }: { scenario: string; appearance?: Appearance }) => {
  const storyAppearance = useAppearance()
  const resolved = appearance ?? storyAppearance
  return (
    <RenderFigure
      image={resolveV3({ scenario, appearance: resolved })}
      title={v3History?.scenarios.find((entry) => entry.id === scenario)?.title ?? scenario}
      appearance={resolved}
      missing={`v3/${scenario}-${resolved}`}
    />
  )
}

const LockedView = () => (
  <div className="at-gallery">
    {v3History === undefined ? null : <p className="at-gallery-description">{v3History.note}</p>}
    <Decisions title="Locked in V3: Ada’ picks (2026-11-13)" decisions={picks} />
    <Decisions title="Locked in V3: carried from V2 and added since" decisions={locked} />
    <section className="at-gallery-row" aria-label="Menu-bar item">
      <h2 className="at-gallery-heading">Menu-bar item · M3 (unchanged in V4)</h2>
      <div className="at-gallery-pair">
        {stateIds.map((state) => (
          <MenuBarItem key={state} state={state} />
        ))}
      </div>
    </section>
    {(v3History?.scenarios ?? []).map((scenario) => (
      <section key={scenario.id} className="at-gallery-row" aria-label={scenario.title}>
        <h2 className="at-gallery-heading">
          {scenario.title}
          <code>{scenario.id}</code>
        </h2>
        <p className="at-gallery-description">{scenario.description}</p>
        <div className="at-gallery-pair">
          <V3Panel scenario={scenario.id} />
        </div>
      </section>
    ))}
  </div>
)

const meta = {
  title: 'AttentionTray/V3',
  component: LockedView,
  parameters: {
    controls: { disable: true },
    docs: {
      description: {
        component:
          'V3 as history: the design #9344 shipped (Ada’ V2 picks L1 A3 H3 S2 M3 with K1–K13) before the native round replaced it with V4. Frozen cacheDisplay renders; appearance follows macOS, override it in the toolbar.',
      },
    },
  },
} satisfies Meta<typeof LockedView>

export default meta
type Story = StoryObj<typeof meta>

/** The locked V3 decisions, the status item in every state and every V3 scenario, frozen. */
export const Locked: Story = {}

/** The V3 busy inbox, light and dark side by side. */
export const LightAndDark: Story = {
  render: () => (
    <div className="at-gallery">
      <div className="at-gallery-pair">
        {appearances.map((appearance) => (
          <V3Panel key={appearance} scenario="busy" appearance={appearance} />
        ))}
      </div>
    </div>
  ),
}
