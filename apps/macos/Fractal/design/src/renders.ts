/**
 * Typed access to the TrayGallery output in `../renders/`: `manifest.json` plus one
 * `<scenario>-<appearance>.png` (@2x) per render. Both are globbed rather than imported so a
 * checkout without renders, or a manifest naming a missing PNG, still builds; consumers get
 * `undefined` and render a placeholder instead.
 */

/** macOS appearance a render was captured in. */
export type Appearance = 'light' | 'dark'

/** Appearances in display order: light first, as in the gallery. */
export const appearances: readonly Appearance[] = ['light', 'dark']

/** Scenario ids TrayGallery renders, in gallery order. The manifest supplies titles and descriptions. */
export const scenarioIds = [
  'busy',
  'answering',
  'quiet',
  'read-only',
  'degraded',
  'unconfigured',
] as const

/** One of the gallery scenarios. */
export type ScenarioId = (typeof scenarioIds)[number]

/** Manifest scenario: id plus the title and one-sentence description TrayGallery declares. */
export interface RenderScenario {
  readonly id: string
  readonly title: string
  readonly description: string
}

/** One PNG in `renders/` for a scenario in one appearance. */
export interface RenderEntry {
  readonly scenario: string
  readonly appearance: Appearance
  readonly file: string
  /** Pixel width of the PNG; the point width is `width / scale`. */
  readonly width: number
  /** Pixel height of the PNG; the point height is `height / scale`. */
  readonly height: number
}

/** Shape of `renders/manifest.json` as written by TrayGallery. */
export interface RenderManifest {
  readonly generator: string
  readonly command: string
  /** Source-closure revision (`src.<7hex>`) the renders were produced from, or `null`. */
  readonly trayRev: string | null
  readonly scale: number
  readonly scenarios: readonly RenderScenario[]
  readonly renders: readonly RenderEntry[]
}

const manifestModules = import.meta.glob<RenderManifest>('../renders/manifest.json', {
  eager: true,
  import: 'default',
})

const pngUrls = import.meta.glob<string>('../renders/*.png', {
  eager: true,
  query: '?url',
  import: 'default',
})

/** The committed manifest, or `undefined` when TrayGallery has not written one yet. */
export const manifest: RenderManifest | undefined = manifestModules['../renders/manifest.json']

/** Pixel density of the renders; 2 unless the manifest says otherwise. */
export const renderScale = manifest?.scale ?? 2

/** Scenario ids offered by the stories: the manifest's, or the gallery order when there is no manifest. */
export const availableScenarioIds: readonly string[] =
  manifest === undefined ? scenarioIds : manifest.scenarios.map((scenario) => scenario.id)

/** Manifest scenario metadata for `id`, absent without a manifest or for an unknown id. */
export const scenarioById = (id: string): RenderScenario | undefined =>
  manifest?.scenarios.find((scenario) => scenario.id === id)

/** A render looked up by scenario and appearance; either half may be missing. */
export interface ResolvedRender {
  readonly scenario: string
  readonly appearance: Appearance
  /** Manifest entry, absent when the manifest has no render for this scenario/appearance. */
  readonly entry: RenderEntry | undefined
  /** Bundled PNG URL, absent when the file named by the manifest is not on disk. */
  readonly url: string | undefined
}

/** Finds the manifest entry and bundled PNG URL for one scenario in one appearance. */
export const resolveRender = ({
  scenario,
  appearance,
}: {
  scenario: string
  appearance: Appearance
}): ResolvedRender => {
  const entry = manifest?.renders.find(
    (render) => render.scenario === scenario && render.appearance === appearance,
  )
  const url = entry === undefined ? undefined : pngUrls[`../renders/${entry.file}`]
  return { scenario, appearance, entry, url }
}
