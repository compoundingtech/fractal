/**
 * Typed access to the TrayGallery output in `../renders/`. Four sets, each with its own manifest,
 * all globbed rather than imported so a checkout without them still builds (consumers get
 * `undefined` and show a placeholder):
 *
 * - `manifest.json` + `<scenario>-<appearance>.png`: the V4 panel rendered with `cacheDisplay`
 *   (no window-server materials), plus the status item in every state as the tinted template
 *   image on a flat bar (`menubar-<state>-<appearance>.png`), and each scenario's height.
 *   Committed.
 * - `capture.json` + `capture/*.png`: real window-server captures (Liquid Glass, the status item)
 *   of every scenario and every status-item state. Committed when a capture run produces them;
 *   absent, every image falls back to the flat render.
 * - `v3/v3.json` + `v3/*.png`: V3 as shipped in #9344 (the inbox-first Form panel), frozen.
 * - `v2/v2.json` + `v2/*.png`: the V2 design exploration Ada picked from, frozen.
 *
 * The variant code that drew V2 and V3 is gone, so they are never regenerated.
 */

/** macOS appearance a render was produced in. */
export type Appearance = 'light' | 'dark'

/** Appearances in display order: light first, as in the gallery. */
export const appearances: readonly Appearance[] = ['light', 'dark']

/** Scenario ids TrayGallery renders, in gallery order. The manifest supplies titles and descriptions. */
export const scenarioIds = [
  'busy',
  'answering',
  'controls',
  'quiet',
  'read-only',
  'degraded',
  'unconfigured',
  'ask-decision',
  'ask-choice',
  'ask-feedback',
  'ask-in-fractal',
  'update-page',
  'ask-sending',
  'page-availability',
  'page-objectives',
  'speech-speaking',
  'speech-in-call',
  'speech-muted',
  'variant-d1',
  'variant-d2',
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

/** An image of the status item alone in one state. */
export interface MenuBarImage {
  readonly state: string
  readonly appearance: Appearance
  readonly file: string
  readonly width: number
  readonly height: number
}

/** One scenario's panel height in points (light appearance). */
export interface PanelHeight {
  readonly scenario: string
  /** As the person sees it: scrolling content capped at the live panel's limit. */
  readonly live: number
  /** Every row and page section in view. */
  readonly full: number
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
  /** Status-item states, in gallery order. */
  readonly menuBarStates: readonly RenderScenario[]
  /** Flat status-item renders: the tinted template image on a plain bar. */
  readonly menuBar: readonly MenuBarImage[]
  /** Each panel scenario's height; absent in manifests that predate V4. */
  readonly heights?: readonly PanelHeight[]
}

/** Shape of `renders/v3/v3.json`: the V3 manifest as it shipped, with a note. File names are relative to `v3/`. */
export interface V3History extends RenderManifest {
  readonly note: string
  readonly source: string
  /** How the V3 heights were measured. */
  readonly heightsSource?: string
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
  readonly panels: readonly RenderEntry[]
  readonly menuBar: readonly MenuBarImage[]
}

/** One frozen V2 image in one appearance. */
export interface V2Render {
  readonly appearance: Appearance
  readonly file: string
  readonly width: number
  readonly height: number
}

/** One image of a V2 option: a panel page, or the status item in one state. */
export interface V2Image {
  readonly title: string
  /** The status-item state, for menu-bar options. */
  readonly state?: string
  readonly renders: readonly V2Render[]
}

/** One option of a V2 dimension with the description it had. */
export interface V2Option {
  readonly id: string
  readonly description: string
  readonly images: readonly V2Image[]
}

/** A V2 dimension: what V2 shipped and what Ada picked. */
export interface V2Dimension {
  readonly id: string
  readonly title: string
  readonly shipped: string
  readonly picked: string
  readonly options: readonly V2Option[]
}

/** Shape of `renders/v2/v2.json`. */
export interface V2History {
  readonly generator: string
  readonly source: string
  readonly note: string
  readonly trayRev: string | null
  readonly scale: number
  readonly scenario: RenderScenario
  readonly dimensions: readonly V2Dimension[]
}

const manifestModules = import.meta.glob<RenderManifest>('../renders/manifest.json', {
  eager: true,
  import: 'default',
})
const captureModules = import.meta.glob<CaptureManifest>('../renders/capture.json', {
  eager: true,
  import: 'default',
})
const v3Modules = import.meta.glob<V3History>('../renders/v3/v3.json', {
  eager: true,
  import: 'default',
})
const v2Modules = import.meta.glob<V2History>('../renders/v2/v2.json', {
  eager: true,
  import: 'default',
})

const pngUrls: Record<string, string> = {
  ...import.meta.glob<string>('../renders/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
  ...import.meta.glob<string>('../renders/capture/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
  ...import.meta.glob<string>('../renders/v3/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
  ...import.meta.glob<string>('../renders/v2/*.png', {
    eager: true,
    query: '?url',
    import: 'default',
  }),
}

/** The committed fallback manifest, or `undefined` when TrayGallery has not written one yet. */
export const manifest: RenderManifest | undefined = manifestModules['../renders/manifest.json']

/** The real-capture manifest (`--capture`). */
export const captureManifest: CaptureManifest | undefined =
  captureModules['../renders/capture.json']

/** V3 as shipped, frozen. */
export const v3History: V3History | undefined = v3Modules['../renders/v3/v3.json']

/** The frozen V2 exploration. */
export const v2History: V2History | undefined = v2Modules['../renders/v2/v2.json']

/** Pixel density of the renders; 2 unless the manifest says otherwise. */
export const renderScale = manifest?.scale ?? 2

/** Scenario ids offered by the stories: the manifest's, or the gallery order when there is no manifest. */
export const availableScenarioIds: readonly string[] =
  manifest === undefined ? scenarioIds : manifest.scenarios.map((scenario) => scenario.id)

/** Manifest scenario metadata for `id`, absent without a manifest or for an unknown id. */
export const scenarioById = (id: string): RenderScenario | undefined =>
  manifest?.scenarios.find((scenario) => scenario.id === id)

/**
 * Where an image came from: the window server (glass, status item), `cacheDisplay` (flat), or
 * frozen history (V3 as shipped, the V2 exploration; flat, from code that no longer exists).
 */
export type ImageSource = 'capture' | 'fallback' | 'history-v3' | 'history'

/** V4's height for a scenario, from the manifest. */
export const heightOf = (scenario: string): PanelHeight | undefined =>
  manifest?.heights?.find((height) => height.scenario === scenario)

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

/** The best image of a scenario: the real capture when there is one, else the `cacheDisplay` render. */
export const resolvePanel = ({
  scenario,
  appearance,
}: {
  scenario: string
  appearance: Appearance
}): ResolvedImage | undefined => {
  const matches = (render: RenderEntry) =>
    render.scenario === scenario && render.appearance === appearance
  const captured = resolve({
    source: 'capture',
    entry: captureManifest?.panels.find(matches),
    scale: captureManifest?.scale ?? 2,
  })
  if (captured?.url !== undefined) return captured
  return resolve({ source: 'fallback', entry: manifest?.renders.find(matches), scale: renderScale })
}

/**
 * The status item in one state: the real capture in the menu bar when there is one, else the
 * flat render of its template image.
 */
export const resolveMenuBar = ({
  state,
  appearance,
}: {
  state: string
  appearance: Appearance
}): ResolvedImage | undefined => {
  const matches = (image: MenuBarImage) => image.state === state && image.appearance === appearance
  const captured = resolve({
    source: 'capture',
    entry: captureManifest?.menuBar.find(matches),
    scale: captureManifest?.scale ?? 2,
  })
  if (captured?.url !== undefined) return captured
  return (
    resolve({ source: 'fallback', entry: manifest?.menuBar.find(matches), scale: renderScale }) ??
    captured
  )
}

/** A frozen V2 image in one appearance. */
export const resolveV2 = ({
  image,
  appearance,
}: {
  image: V2Image
  appearance: Appearance
}): ResolvedImage | undefined =>
  resolve({
    source: 'history',
    entry: image.renders.find((render) => render.appearance === appearance),
    scale: v2History?.scale ?? 2,
  })

/** A frozen V3 panel scenario in one appearance (file names are relative to `v3/`). */
export const resolveV3 = ({
  scenario,
  appearance,
}: {
  scenario: string
  appearance: Appearance
}): ResolvedImage | undefined => {
  const entry = v3History?.renders.find(
    (render) => render.scenario === scenario && render.appearance === appearance,
  )
  return resolve({
    source: 'history-v3',
    entry: entry === undefined ? undefined : { ...entry, file: `v3/${entry.file}` },
    scale: v3History?.scale ?? 2,
  })
}

/** Menu-bar states, in gallery order. */
export const menuBarStates: readonly RenderScenario[] =
  manifest?.menuBarStates ?? captureManifest?.states ?? []

/** Find a menu-bar state's metadata. */
export const menuBarStateById = (id: string): RenderScenario | undefined =>
  menuBarStates.find((state) => state.id === id)

/** The command that regenerates a set. */
export const regenerateCommand = (source: ImageSource): string => {
  switch (source) {
    case 'capture':
      return captureManifest?.command ?? 'devenv tasks run attention-tray:gallery-capture-vm'
    case 'fallback':
      return manifest?.command ?? 'devenv tasks run attention-tray:gallery'
    case 'history-v3':
      return 'not regenerated: V3’s views are gone (git history has them)'
    case 'history':
      return 'not regenerated: the V2 variant code is gone (git history has it)'
  }
}
