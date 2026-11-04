/** The host build supplies emitted URLs, including recursive static imports and CSS. */
export const readLazyChunkAssets = (feature: string): readonly string[] => {
  const source = document.getElementById('fractal-lazy-chunk-assets')?.textContent
  if (source === undefined || source === null) return [] // Development has no emitted chunks.
  const manifest: unknown = JSON.parse(source)
  if (typeof manifest !== 'object' || manifest === null || !(feature in manifest)) return []
  const assets: unknown = Reflect.get(manifest, feature)
  return Array.isArray(assets) && assets.every((asset: unknown) => typeof asset === 'string') ? assets : []
}

const refreshAsset = async (url: string): Promise<void> => {
  const response = await fetch(url, { cache: 'reload', credentials: 'same-origin' })
  if (!response.ok) throw new Error(`Could not refresh lazy asset (${response.status})`)
  await response.arrayBuffer()
}

/** Vite remembers failed CSS preloads; a later import can resolve without its stylesheet. */
const repairStylesheets = async (assets: readonly string[]): Promise<void> => {
  for (const asset of assets) {
    const url = new URL(asset, document.baseURI)
    if (!url.pathname.endsWith('.css')) continue
    const existing = [...document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')]
      .find(link => link.href === url.href)
    // Even an aborted stylesheet can retain a non-null opaque sheet.
    await refreshAsset(url.href)
    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.crossOrigin = ''
    link.href = url.href
    const ready = new Promise<void>((resolve, reject) => {
      link.onload = () => resolve()
      link.onerror = () => reject(new Error('Could not reload lazy stylesheet'))
    })
    if (existing) existing.replaceWith(link)
    else document.head.append(link)
    await ready
  }
}

export interface LazyChunk<TModule> {
  readonly load: () => Promise<TModule>
  readonly retry: () => Promise<TModule>
  readonly reload: () => Promise<void>
  readonly recovery: 'retry' | 'reload'
}

/** Retry once in-document: https://github.com/whatwg/html/pull/10327.
 * Older engines retain rejected modules; a second failure requires a new document.
 */
export const createLazyChunk = <TModule>({ load: importer, assets = () => [] }: {
  readonly load: () => Promise<TModule>
  readonly assets?: () => readonly string[]
}): LazyChunk<TModule> => {
  let request: Promise<TModule> | undefined
  let retryRequest: Promise<TModule> | undefined
  let failures = 0
  const attempt = (retry: boolean): Promise<TModule> => (retry ? repairStylesheets(assets()) : Promise.resolve())
    .then(importer).catch(error => { failures += 1; throw error })
  return {
    load: () => request ??= attempt(false),
    retry: () => {
      retryRequest ??= attempt(true).finally(() => { retryRequest = undefined })
      return request = retryRequest
    },
    get recovery() { return failures >= 2 ? 'reload' : 'retry' },
    reload: async () => {
      // WebKit can retain failed modulepreload resources across reload. Ordinary fetch
      // repairs that resource cache without evaluating a second module instance.
      // A removed old deployment asset must not prevent navigation to current HTML.
      await Promise.allSettled(assets().map(refreshAsset))
      window.location.reload()
    },
  }
}
