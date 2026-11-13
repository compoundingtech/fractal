/**
 * Attention tray V2, kept as history: the open variant dimensions Ada picked from, each
 * option rendered one change at a time from the V2 design. The variant code is gone; the images
 * are frozen in `renders/v2/` and never regenerated. V3 is what the tray ships.
 */
import type { Meta, StoryObj } from '@storybook/react'

import { useAppearance } from './appearance.tsx'
import { RenderFigure } from './RenderedPanel.tsx'
import { resolveV2, v2History, type V2Dimension } from './renders.ts'

/** Decisions every V2 combination shared, as they stood when the picks were made. */
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

/** One dimension's options side by side, the pick marked. */
const DimensionRow = ({ dimension }: { dimension: V2Dimension }) => {
  const appearance = useAppearance()
  return (
    <section className="at-gallery-row" aria-label={dimension.title}>
      <h2 className="at-gallery-heading">
        {dimension.id} · {dimension.title}
        <code>
          V2 shipped {dimension.shipped} · picked {dimension.picked}
        </code>
      </h2>
      <div className="at-gallery-pair">
        {dimension.options.map((option) => (
          <div
            key={option.id}
            className="at-option"
            data-picked={option.id === dimension.picked ? 'true' : undefined}
          >
            <h3 className="at-option-heading">
              {option.id}
              {option.id === dimension.picked ? <span className="at-pick">Picked</span> : null}
            </h3>
            <p className="at-gallery-description">{option.description}</p>
            <div className="at-gallery-pair">
              {option.images.map((image) => (
                <RenderFigure
                  key={image.title}
                  image={resolveV2({ image, appearance })}
                  title={image.title}
                  appearance={appearance}
                  missing={`v2 ${option.id} ${image.title} ${appearance}`}
                />
              ))}
            </div>
          </div>
        ))}
      </div>
    </section>
  )
}

const HistoryView = () =>
  v2History === undefined ? (
    <div className="at-gallery">
      <p>renders/v2/v2.json is missing.</p>
    </div>
  ) : (
    <div className="at-gallery">
      <section className="at-gallery-row" aria-label="About V2">
        <h2 className="at-gallery-heading">
          V2 exploration (history)
          <code>{v2History.trayRev}</code>
        </h2>
        <p className="at-gallery-description">
          {v2History.note} Scenario: {v2History.scenario.title} — {v2History.scenario.description}
        </p>
        <p className="at-gallery-description">
          Ada picked{' '}
          <strong>{v2History.dimensions.map((dimension) => dimension.picked).join(' ')}</strong> on
          2026-11-13; V3 ships them.
        </p>
      </section>
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
      {v2History.dimensions.map((dimension) => (
        <DimensionRow key={dimension.id} dimension={dimension} />
      ))}
    </div>
  )

const meta = {
  title: 'AttentionTray/V2',
  component: HistoryView,
  parameters: {
    controls: { disable: true },
    docs: {
      description: {
        component:
          'History: the V2 exploration Ada picked L1 A3 H3 S2 M3 from. Each option is a frozen flat render of the V2 design with one dimension changed; nothing here is regenerated. AttentionTray/V3 shows what ships.',
      },
    },
  },
} satisfies Meta<typeof HistoryView>

export default meta
type Story = StoryObj<typeof meta>

/** Every V2 option per dimension, in the current appearance; the pick is marked. */
export const Exploration: Story = {}
