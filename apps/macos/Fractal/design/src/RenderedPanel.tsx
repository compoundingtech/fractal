import { useAppearance } from './appearance.tsx'
import {
  captureManifest,
  manifest,
  regenerateCommand,
  resolvePanel,
  scenarioById,
  v2History,
  v3History,
  type Appearance,
  type ImageSource,
  type ResolvedImage,
} from './renders.ts'

/** Point size used for the placeholder when no manifest has dimensions for an image. */
const placeholderSize = { width: 400, height: 320 }

const sourceLabel: Record<ImageSource, string> = {
  capture: 'Real capture · window server, Liquid Glass',
  fallback: 'Fallback · cacheDisplay, no materials, controls drawn inactive',
  'history-v3': 'V3 history · cacheDisplay, frozen',
  history: 'V2 history · cacheDisplay, frozen',
}

const sourceRev: Record<ImageSource, string | null | undefined> = {
  capture: captureManifest?.trayRev,
  fallback: manifest?.trayRev,
  'history-v3': v3History?.trayRev,
  history: v2History?.trayRev,
}

/** Props of {@link RenderFigure}. */
export interface RenderFigureProps {
  readonly image: ResolvedImage | undefined
  readonly title: string
  readonly appearance: Appearance
  /** What is missing when there is no image, e.g. `busy-dark`. */
  readonly missing: string
}

/**
 * One generated image at point size on the window background of its appearance, captioned with
 * its title, appearance and source (real capture, flat fallback or frozen V2). A missing image
 * shows a labelled placeholder of the same footprint with the command that regenerates it.
 */
export const RenderFigure = ({ image, title, appearance, missing }: RenderFigureProps) => {
  const size = image ?? placeholderSize
  const rev = sourceRev[image?.source ?? 'fallback']
  return (
    <figure className="at-panel" data-appearance={appearance} data-source={image?.source}>
      {image?.url === undefined ? (
        <div
          className="at-placeholder"
          role="img"
          aria-label={`Missing render: ${missing}`}
          style={{ width: size.width, height: size.height }}
        >
          <strong>No render for {missing}</strong>
          <span>
            {image === undefined
              ? 'No manifest lists it yet.'
              : `renders/${image.file} is not on disk.`}
          </span>
          <code>{regenerateCommand(image?.source ?? 'fallback')}</code>
        </div>
      ) : (
        <img
          className="at-render"
          src={image.url}
          alt={`${title} (${appearance})`}
          width={image.width}
          height={image.height}
        />
      )}
      <figcaption className="at-caption" style={{ maxWidth: Math.max(size.width, 320) }}>
        <span className="at-caption-title">{title}</span>
        <span>{appearance}</span>
        {image === undefined ? null : (
          <span className="at-source" data-source={image.source}>
            {sourceLabel[image.source]}
          </span>
        )}
        {/* Glass differs between macOS releases: a capture says which one drew it. */}
        {image?.source === 'capture' && captureManifest !== undefined ? (
          <span>{captureManifest.system.replace(/^Version /, 'macOS ')}</span>
        ) : null}
        <span>{rev ?? 'unversioned source'}</span>
      </figcaption>
    </figure>
  )
}

/** Props of {@link RenderedPanel}. */
export interface RenderedPanelProps {
  /** Scenario id from the render manifest (`busy`, `quiet`, …). */
  readonly scenario: string
  /** Forced appearance; by default the Storybook appearance (macOS unless overridden). */
  readonly appearance?: Appearance
  /** Caption title; the scenario title by default. */
  readonly title?: string
}

/** The best image of a panel: the real capture when there is one, else the flat render. */
export const RenderedPanel = ({ scenario, appearance, title }: RenderedPanelProps) => {
  const storyAppearance = useAppearance()
  const resolvedAppearance = appearance ?? storyAppearance
  return (
    <RenderFigure
      image={resolvePanel({ scenario, appearance: resolvedAppearance })}
      title={title ?? scenarioById(scenario)?.title ?? scenario}
      appearance={resolvedAppearance}
      missing={`${scenario}-${resolvedAppearance}`}
    />
  )
}
