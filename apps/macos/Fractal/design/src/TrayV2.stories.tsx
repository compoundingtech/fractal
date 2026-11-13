/**
 * Attention tray V2: the native macOS 26/27 panel after the HIG review, with the variant
 * dimensions still open for Johannes to pick by ID.
 *
 * Locked (`K1`–`K12`, listed in the story) apply to every combination; open dimensions are
 * L information architecture, A ask row, H header, S objective history, M menu-bar item.
 */
import type { Meta, StoryObj } from '@storybook/react'
import { useState } from 'react'

import { useAppearance } from './appearance.tsx'
import { MenuBarItem } from './MenuBarItem.tsx'
import { RenderedPanel } from './RenderedPanel.tsx'
import { appearances, exploreManifest, type Appearance } from './renders.ts'

/** Decisions every V2 combination shares; not up for a pick. */
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
    text: 'One leading edge: title, chips and footer share the Form’s content inset (30 pt).',
  },
  {
    id: 'K4',
    text: 'At most one prominent button: the first answerable ask’s recommended answer (never a decline), while nothing is being typed. No answer takes Return: answers are irreversible and the next ask moves into place after a commit.',
  },
  {
    id: 'K5',
    text: 'Every control is always in place (no hover-only actions): Mark Read, Open in Fractal, the A1 Answer menu; keyboard and VoiceOver reach all of them.',
  },
  {
    id: 'K6',
    text: '“Nothing to Do” is an answer button beside the other answers; Read Aloud is icon-only in the ask row and becomes “Stop Speaking” while that ask is read.',
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
    text: 'Status item: one template glyph (plus the count in M1/M3); offline swaps the glyph for the sync-problem symbol; objectives stay in the panel (no gauge glyph).',
  },
  {
    id: 'K12',
    text: 'Histograms: Swift Charts bars per hour; spent hours carry a cross, unmeasured hours a grey stub, the nearly-spent zone a dashed rule (full) or shaded band (mini).',
  },
]

/** The HIG review’s pick per open dimension (V2 review), shown beside the shipped dot. */
const reviewPicks: Readonly<Record<string, string>> = {
  L: 'L1',
  A: 'A3',
  H: 'H3',
  S: 'S2',
  M: 'M3',
}

/** One variant option with the description TrayDesignCatalog gives it. */
interface Variant {
  readonly id: string
  readonly description: string
}

/** Colocated variant picker: short IDs, the selected one's description inline. */
const VariantPicker = ({
  label,
  variants,
  value,
  current,
  recommended,
  onChange,
}: {
  label: string
  variants: readonly Variant[]
  value: string
  current: string | undefined
  recommended?: string | undefined
  onChange: (id: string) => void
}) => (
  <div className="at-picker" role="group" aria-label={label}>
    <div className="at-picker-label">{label}</div>
    <div className="at-picker-row">
      <div className="at-segmented">
        {variants.map((variant) => (
          <button
            key={variant.id}
            type="button"
            aria-pressed={value === variant.id}
            title={variant.description}
            onClick={() => onChange(variant.id)}
          >
            {variant.id}
            {variant.id === current ? (
              <span className="at-current-dot" aria-label="current" />
            ) : null}
            {variant.id === recommended ? (
              <span className="at-review-pick" aria-label="HIG review pick">
                ★
              </span>
            ) : null}
          </button>
        ))}
      </div>
      <span className="at-picker-description">
        ← {variants.find((variant) => variant.id === value)?.description}
      </span>
    </div>
  </div>
)

const dimension = (id: string): readonly Variant[] =>
  exploreManifest?.dimensions.find((candidate) => candidate.id === id)?.options ?? []

const scenarios = [
  {
    id: 'busy',
    description: 'Busy: urgent decision, choice, yes/no, in-Fractal ask; an objective missed',
    menuBar: 'urgent',
  },
  {
    id: 'quiet',
    description: 'Quiet: nothing open; away, urgent interruptions only',
    menuBar: 'idle',
  },
  {
    id: 'degraded',
    description: 'Fractal unreachable: every section stale, last data kept',
    menuBar: 'offline',
  },
] as const

const pages: readonly Variant[] = [
  { id: 'inbox', description: 'Inbox page' },
  { id: 'availability', description: 'Availability page' },
  { id: 'objectives', description: 'Objectives page' },
]

const ExplorationView = () => {
  const current = exploreManifest?.current ?? {}
  const [layout, setLayout] = useState(current['L'] ?? 'L1')
  const [askRow, setAskRow] = useState(current['A'] ?? 'A2')
  const [header, setHeader] = useState(current['H'] ?? 'H3')
  const [chart, setChart] = useState(current['S'] ?? 'S1')
  const [menuBar, setMenuBar] = useState(current['M'] ?? 'M1')
  const [page, setPage] = useState('inbox')
  const [scenario, setScenario] = useState<(typeof scenarios)[number]['id']>('busy')
  const [compare, setCompare] = useState(false)
  const appearance = useAppearance()
  const shown: readonly Appearance[] = compare ? appearances : [appearance]
  const currentChart = current['S'] ?? 'S1'
  const pageShown = layout === 'L2' ? page : null
  // Histogram variants are rendered on the Objectives page; elsewhere the panel uses the current one.
  const chartInPanel = layout === 'L2' && page === 'objectives' ? chart : currentChart
  const panel = `${layout}-${askRow}-${header}-${chartInPanel}`
  const objectivesPanel = `L2-${current['A'] ?? 'A2'}-${header}-${chart}`
  const menuBarState = scenarios.find((candidate) => candidate.id === scenario)?.menuBar ?? 'urgent'
  const selection = `${layout} + ${askRow} + ${header} + ${chart} + ${menuBar}`

  if (exploreManifest === undefined) {
    return (
      <div className="at-gallery">
        <p>
          renders/explore.json is missing: run{' '}
          <code>devenv tasks run attention-tray:gallery-explore</code> on a Mac.
        </p>
      </div>
    )
  }

  return (
    <div className="at-gallery">
      <section className="at-gallery-row" aria-label="Locked in V2">
        <h2 className="at-gallery-heading">Locked in V2 (every combination)</h2>
        <ul className="at-sources">
          {locked.map((decision) => (
            <li key={decision.id}>
              <strong>{decision.id}</strong> {decision.text}
            </li>
          ))}
        </ul>
      </section>
      <div className="at-controls">
        <h2 className="at-gallery-heading">Open: pick one per dimension by ID</h2>
        <VariantPicker
          label="L · Information architecture"
          variants={dimension('L')}
          value={layout}
          current={current['L']}
          recommended={reviewPicks['L']}
          onChange={setLayout}
        />
        {layout === 'L2' ? (
          <VariantPicker
            label="L2 page"
            variants={pages}
            value={page}
            current="inbox"
            onChange={setPage}
          />
        ) : null}
        <VariantPicker
          label="A · Ask row"
          variants={dimension('A')}
          value={askRow}
          current={current['A']}
          recommended={reviewPicks['A']}
          onChange={setAskRow}
        />
        <VariantPicker
          label="H · Header"
          variants={dimension('H')}
          value={header}
          current={current['H']}
          recommended={reviewPicks['H']}
          onChange={setHeader}
        />
        <VariantPicker
          label="S · Objective history"
          variants={dimension('S')}
          value={chart}
          current={current['S']}
          recommended={reviewPicks['S']}
          onChange={setChart}
        />
        <VariantPicker
          label="M · Menu-bar item"
          variants={dimension('M')}
          value={menuBar}
          current={current['M']}
          recommended={reviewPicks['M']}
          onChange={setMenuBar}
        />
        <VariantPicker
          label="Scenario"
          variants={scenarios}
          value={scenario}
          current={undefined}
          onChange={(id) => setScenario(id as (typeof scenarios)[number]['id'])}
        />
        <label className="at-compare">
          <input
            type="checkbox"
            checked={compare}
            onChange={(event) => setCompare(event.target.checked)}
          />
          Compare light and dark side by side
        </label>
        <p className="at-selection">
          Selected: <code>{selection}</code> · a dot marks what the tray ships today, ★ the HIG
          review&apos;s pick · tell Johannes&apos;s agent the IDs.
        </p>
      </div>
      {shown.map((shownAppearance) => (
        <section key={shownAppearance} className="at-gallery-row" aria-label={shownAppearance}>
          <div className="at-gallery-pair">
            <div className="at-stack">
              <MenuBarItem menuBar={menuBar} state={menuBarState} appearance={shownAppearance} />
              <RenderedPanel
                scenario={scenario}
                appearance={shownAppearance}
                panel={panel}
                page={pageShown}
                title={`${panel}${pageShown === null ? '' : ` · ${pageShown}`}`}
              />
            </div>
            {panel === objectivesPanel && pageShown === 'objectives' ? null : (
              <RenderedPanel
                scenario={scenario}
                appearance={shownAppearance}
                panel={objectivesPanel}
                page="objectives"
                title={`Objectives · ${chart} (as on the L2 page; L1 expands it in place from its chip)`}
              />
            )}
          </div>
        </section>
      ))}
    </div>
  )
}

const meta = {
  title: 'AttentionTray/V2',
  component: ExplorationView,
  parameters: {
    docs: {
      description: {
        component:
          'V2 of the native rebuild, after the HIG review. Locked decisions K1–K12 hold for every combination; pick L/A/H/S/M by ID. Each image is a real window-server capture where one exists (labelled), else a flat cacheDisplay render.',
      },
    },
  },
} satisfies Meta<typeof ExplorationView>

export default meta
type Story = StoryObj<typeof meta>

/** Pick L/A/H/S/M and a scenario; the real SwiftUI render of that combination shows. */
export const Exploration: Story = {}

/** What the tray ships in V2 (the dotted options), busy inbox. */
export const Current: Story = {
  render: () => (
    <div className="at-gallery">
      <div className="at-stack">
        <MenuBarItem menuBar={exploreManifest?.current['M'] ?? 'M1'} state="urgent" />
        <RenderedPanel scenario="busy" />
      </div>
    </div>
  ),
}
