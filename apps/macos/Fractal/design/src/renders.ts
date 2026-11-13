/**
 * Typed access to the TrayGallery output in `../renders/`. Three generated sets, each with its own
 * manifest, all globbed rather than imported so a checkout without them still builds (consumers
 * get `undefined` and show a placeholder):
 *
 * - `manifest.json` + `<scenario>-<appearance>.png`: the current design, rendered with
 *   `cacheDisplay` (no window-server materials), plus every menu-bar variant × state as the
 *   tinted template image on a flat bar (`menubar-*.png`). Committed.
 * - `capture.json` + `capture/*.png`: real window-server captures (Liquid Glass, the status item)
 *   of the current design, the curated variants and every menu-bar variant × state. Committed.
 * - `explore.json` + `explore/*.png`: every layout × ask-row × header combination via
 *   `cacheDisplay`, for the Exploration story. Generated locally, not committed.
 */

/** macOS appearance a render was produced in. */
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
  'speech-speaking',
  'speech-in-call',
  'speech-muted',
  'notice-speaking',
  'notice-in-call',
  'notice-answered',
] as const

/** One of the gallery scenarios. */
export type ScenarioId = (typeof scenarioIds)[number]

/** Manifest scenario or state: id plus the title and one-sentence description TrayGallery declares. */
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

/** An image of the status item alone for one menu-bar variant in one state. */
export interface MenuBarImage {
  readonly menuBar: string
  readonly state: string
  readonly appearance: Appearance
  readonly file: string
  readonly width: number
  readonly height: number
}

/** Shape of `renders/manifest.json` as written by TrayGallery. */
export interface RenderManifest {
  readonly generator: string
  readonly command: string
  /** Source-closure revision (`src.<7hex>`) the renders were produced from, or `null`. */
  readonly trayRev: string | null
  readonly scale: number
  /** The design the renders show (`L1-A2-H3-S1`). */
  readonly panel?: string
  readonly scenarios: readonly RenderScenario[]
  readonly renders: readonly RenderEntry[]
  /** Status-item states, in gallery order (absent in manifests from before the flat menu-bar renders). */
  readonly menuBarStates?: readonly RenderScenario[]
  /** Flat status-item renders: the tinted template image on a plain bar. */
  readonly menuBar?: readonly MenuBarImage[]
}

/** A panel render for one design combination (`L1-A2-H3`) and, for the segmented layout, one page. */
export interface PanelRender {
  readonly panel: string
  readonly page: string | null
  readonly scenario: string
  readonly appearance: Appearance
  readonly file: string
  readonly width: number
  readonly height: number
}

/** One option of a variant dimension, as `TrayDesignCatalog` describes it. */
export interface VariantOption {
  readonly id: string
  readonly description: string
}

/** A variant dimension (`L`, `A`, `H`, `M`) with its options. */
export interface VariantDimension {
  readonly id: string
  readonly title: string
  readonly options: readonly VariantOption[]
}

/** Shape of `renders/explore.json`. */
export interface ExploreManifest {
  readonly generator: string
  readonly command: string
  readonly source: string
  readonly trayRev: string | null
  readonly scale: number
  /** The design the running tray ships, by dimension id. */
  readonly current: Readonly<Record<string, string>>
  readonly dimensions: readonly VariantDimension[]
  readonly scenarios: readonly RenderScenario[]
  readonly renders: readonly PanelRender[]
}

/** Shape of `renders/capture.json`. */
export interface CaptureManifest {
  readonly generator: string
  readonly command: string
  readonly source: string
  readonly trayRev: string | null
  readonly system: string
  readonly scale: number
  readonly states: readonly RenderScenario[]
  readonly panels: readonly PanelRender[]
  readonly menuBar: readonly MenuBarImage[]
}

const manifestModules = import.meta.glob<RenderManifest>('../renders/manifest.json', {
  eager: true,
  import: 'default',
})
const exploreModules = import.meta.glob<ExploreManifest>('../renders/explore.json', {
  eager: true,
  import: 'default',
})
const captureModules = import.meta.glob<CaptureManifest>('../renders/capture.json', {
  eager: true,
  import: 'default',
})

const pngUrls: Record<string, string> = {
  ...import.meta.glob<string>('../renders/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
  ...import.meta.glob<string>('../renders/explore/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
  ...import.meta.glob<string>('../renders/capture/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
}

/** The committed fallback manifest, or `undefined` when TrayGallery has not written one yet. */
export const manifest: RenderManifest | undefined = manifestModules['../renders/manifest.json']

/** The local exploration manifest (`--explore`), absent in CI and fresh checkouts. */
export const exploreManifest: ExploreManifest | undefined =
  exploreModules['../renders/explore.json']

/** The real-capture manifest (`--capture`). */
export const captureManifest: CaptureManifest | undefined =
  captureModules['../renders/capture.json']

/** Pixel density of the renders; 2 unless the manifest says otherwise. */
export const renderScale = manifest?.scale ?? 2

/** Scenario ids offered by the stories: the manifest's, or the gallery order when there is no manifest. */
export const availableScenarioIds: readonly string[] =
  manifest === undefined ? scenarioIds : manifest.scenarios.map((scenario) => scenario.id)

/** Manifest scenario metadata for `id`, absent without a manifest or for an unknown id. */
export const scenarioById = (id: string): RenderScenario | undefined =>
  manifest?.scenarios.find((scenario) => scenario.id === id)

/** Where an image came from: the window server (glass, status item) or `cacheDisplay` (flat). */
export type ImageSource = 'capture' | 'fallback'

/** A render or capture resolved to a bundled URL and point size. */
export interface ResolvedImage {
  readonly source: ImageSource
  readonly file: string
  /** Bundled PNG URL, absent when the manifest names a file that is not on disk. */
  readonly url: string | undefined
  readonly width: number
  readonly height: number
}

const resolve = ({
  source,
  entry,
  scale,
}: {
  source: ImageSource
  entry: { file: string; width: number; height: number } | undefined
  scale: number
}): ResolvedImage | undefined =>
  entry === undefined
    ? undefined
    : {
        source,
        file: entry.file,
        url: pngUrls[`../renders/${entry.file}`],
        width: entry.width / scale,
        height: entry.height / scale,
      }

/** The current design's panel id (`L1-A2-H3-S1`), from whichever manifest knows it. */
export const currentPanel: string | undefined =
  manifest?.panel ??
  (exploreManifest === undefined
    ? undefined
    : `${exploreManifest.current.L}-${exploreManifest.current.A}-${exploreManifest.current.H}-${exploreManifest.current.S}`)

/**
 * The best image of a panel combination: the real capture when there is one, else the
 * `cacheDisplay` render. `panel` undefined means the current design (committed renders).
 */
export const resolvePanel = ({
  panel,
  page,
  scenario,
  appearance,
}: {
  panel?: string | undefined
  page?: string | null | undefined
  scenario: string
  appearance: Appearance
}): ResolvedImage | undefined => {
  const wantedPanel = panel ?? currentPanel
  const pageMatches = (candidate: string | null) => (page ?? null) === candidate
  const capture = captureManifest?.panels.find(
    (render) =>
      (wantedPanel === undefined || render.panel === wantedPanel) &&
      pageMatches(render.page) &&
      render.scenario === scenario &&
      render.appearance === appearance,
  )
  const captured = resolve({
    source: 'capture',
    entry: capture,
    scale: captureManifest?.scale ?? 2,
  })
  if (captured?.url !== undefined) return captured
  if (panel === undefined || panel === currentPanel) {
    const committed = manifest?.renders.find(
      (render) =>
        render.scenario === scenario && render.appearance === appearance && pageMatches(null),
    )
    const fallback = resolve({ source: 'fallback', entry: committed, scale: renderScale })
    if (fallback !== undefined) return fallback
  }
  const explored = exploreManifest?.renders.find(
    (render) =>
      render.panel === wantedPanel &&
      pageMatches(render.page) &&
      render.scenario === scenario &&
      render.appearance === appearance,
  )
  return resolve({ source: 'fallback', entry: explored, scale: exploreManifest?.scale ?? 2 })
}

/**
 * The status item for one menu-bar variant in one state: the real capture in the menu bar when
 * there is one, else the flat render of its template image.
 */
export const resolveMenuBar = ({
  menuBar,
  state,
  appearance,
}: {
  menuBar: string
  state: string
  appearance: Appearance
}): ResolvedImage | undefined => {
  const matches = (image: MenuBarImage) =>
    image.menuBar === menuBar && image.state === state && image.appearance === appearance
  const captured = resolve({
    source: 'capture',
    entry: captureManifest?.menuBar.find(matches),
    scale: captureManifest?.scale ?? 2,
  })
  if (captured?.url !== undefined) return captured
  return (
    resolve({ source: 'fallback', entry: manifest?.menuBar?.find(matches), scale: renderScale }) ??
    captured
  )
}

/** Menu-bar states, in gallery order. */
export const menuBarStates: readonly RenderScenario[] =
  manifest?.menuBarStates ?? captureManifest?.states ?? []

/** Find a menu-bar state's metadata. */
export const menuBarStateById = (id: string): RenderScenario | undefined =>
  menuBarStates.find((state) => state.id === id)

/** The command that regenerates a set. */
export const regenerateCommand = (source: ImageSource): string =>
  source === 'capture'
    ? (captureManifest?.command ?? 'devenv tasks run attention-tray:gallery-capture')
    : (manifest?.command ?? 'devenv tasks run attention-tray:gallery')
