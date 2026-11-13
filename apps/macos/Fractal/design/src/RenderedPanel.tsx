import { manifest, renderScale, resolveRender, scenarioById, type Appearance } from './renders.ts'

/** Point size used for the placeholder when the manifest has no dimensions for a render. */
const placeholderSize = { width: 400, height: 320 }

const missingReason = ({ file }: { file: string | undefined }): string => {
  if (manifest === undefined) return 'renders/manifest.json is missing.'
  if (file === undefined) return 'The manifest lists no render for this scenario and appearance.'
  return `renders/${file} is missing.`
}

/** Props of {@link RenderedPanel}. */
export interface RenderedPanelProps {
  /** Scenario id from the render manifest (`busy`, `quiet`, …). */
  readonly scenario: string
  readonly appearance: Appearance
}

/**
 * One TrayGallery render at point size (pixel size / scale) on the macOS window background of its
 * appearance, captioned with scenario title, appearance and the tray source revision. A render the
 * manifest or disk does not have shows a labelled placeholder of the same footprint.
 */
export const RenderedPanel = ({ scenario, appearance }: RenderedPanelProps) => {
  const render = resolveRender({ scenario, appearance })
  const title = scenarioById(scenario)?.title ?? scenario
  const size =
    render.entry === undefined
      ? placeholderSize
      : { width: render.entry.width / renderScale, height: render.entry.height / renderScale }

  return (
    <figure className="at-panel" data-appearance={appearance}>
      {render.url === undefined ? (
        <div
          className="at-placeholder"
          role="img"
          aria-label={`Missing render: ${scenario} ${appearance}`}
          style={{ width: size.width, height: size.height }}
        >
          <strong>No render for {`${scenario}-${appearance}`}</strong>
          <span>{missingReason({ file: render.entry?.file })}</span>
          <code>{manifest?.command ?? 'devenv tasks run attention-tray:gallery'}</code>
        </div>
      ) : (
        <img
          className="at-render"
          src={render.url}
          alt={`${title} (${appearance})`}
          width={size.width}
          height={size.height}
        />
      )}
      <figcaption className="at-caption" style={{ maxWidth: size.width }}>
        <span className="at-caption-title">{title}</span>
        <span>{appearance}</span>
        <span>{manifest?.trayRev ?? 'unversioned source'}</span>
      </figcaption>
    </figure>
  )
}
