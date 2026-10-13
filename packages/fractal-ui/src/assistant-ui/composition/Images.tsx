import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { VisuallyHidden } from 'react-aria'
import { Button, Dialog, ListBox, ListBoxItem, Modal, ModalOverlay, Tooltip, TooltipTrigger, type Selection } from 'react-aria-components'
import { Icon } from './Icons'
import { ThemePortal } from '../taste/ThemePortal'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'

export type TranscriptImageMediaType = 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp'
/** What the host's loader observed. A rejected `load()` is a transient failure the reader can retry. */
export type TranscriptImageLoad =
  | { readonly _tag: 'Loaded'; readonly blob: Blob }
  /** The source no longer serves the bytes, e.g. an invalidated content ref or an expired blob. */
  | { readonly _tag: 'Gone' }
  | { readonly _tag: 'TooLarge'; readonly bytes: number }
  /** The sniffed bytes are not one of the four image types the page renders. */
  | { readonly _tag: 'Unsupported' }
/** One image in the thread. The kit never sees a ref or a credential: `load` resolves the bytes. */
export interface TranscriptImage {
  /** Stable per transcript position; the viewer keeps its place by this id when the set changes. */
  readonly id: string
  /** The transcript item the image belongs to: a prompt, a work call, or an answer. */
  readonly anchor: string
  readonly origin: 'input' | 'tool' | 'prose'
  readonly load: () => Promise<TranscriptImageLoad>
  /** Natural size, when known before the bytes arrive; without it the space is reserved at 4:3. */
  readonly width?: number
  readonly height?: number
  readonly bytes?: number
  readonly mediaType?: TranscriptImageMediaType
  /** `title` is the row's own words and the image's text alternative; missing `meta` facts drop out. */
  readonly caption: { readonly title: string; readonly meta: readonly string[] }
  /** Host download for bytes the page does not show (too large, unsupported type). */
  readonly onDownload?: () => void
}
type Source =
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Ready'; readonly url: string }
  | { readonly _tag: 'Failed' }
  | { readonly _tag: 'Gone' }
  | { readonly _tag: 'TooLarge'; readonly bytes: number }
  | { readonly _tag: 'Unsupported' }
const loading: Source = { _tag: 'Loading' }
/** At most four loads in flight, matching the server's per-conversation read slots. */
const maxInFlight = 4

/** Per-thread object-URL cache: one load per image id, shared by thumbnails and the viewer. */
class ImageStore {
  private readonly sources = new Map<string, Source>()
  private readonly listeners = new Set<() => void>()
  private readonly queue: (() => void)[] = []
  private inFlight = 0
  private generation = 0
  private releaseTimer: ReturnType<typeof setTimeout> | undefined
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
  get = (id: string): Source | undefined => this.sources.get(id)
  request(image: TranscriptImage) {
    if (this.sources.has(image.id)) return
    this.set(image.id, loading)
    const generation = this.generation
    const run = () => {
      this.inFlight += 1
      void image.load().then(
        (result): Source => result._tag === 'Loaded' ? { _tag: 'Ready', url: URL.createObjectURL(result.blob) } : result,
        (): Source => ({ _tag: 'Failed' }),
      ).then(source => {
        if (generation !== this.generation || this.sources.get(image.id) !== loading) {
          if (source._tag === 'Ready') URL.revokeObjectURL(source.url)
          return
        }
        this.set(image.id, source)
      }).finally(() => {
        this.inFlight -= 1
        this.queue.shift()?.()
      })
    }
    if (this.inFlight < maxInFlight) run()
    else this.queue.push(run)
  }
  retry(image: TranscriptImage) {
    this.drop(image.id)
    this.request(image)
  }
  /** The bytes did not decode: the image is shown as a failed load, never as an empty box. */
  fail(id: string) {
    const source = this.sources.get(id)
    if (source?._tag !== 'Ready') return
    URL.revokeObjectURL(source.url)
    this.set(id, { _tag: 'Failed' })
  }
  retain() { clearTimeout(this.releaseTimer) }
  /** Deferred so a remount in the same task keeps the cache. */
  release() {
    this.releaseTimer = setTimeout(() => {
      this.generation += 1
      for (const id of [...this.sources.keys()]) this.drop(id)
      this.queue.length = 0
      this.emit()
    }, 0)
  }
  private drop(id: string) {
    const source = this.sources.get(id)
    if (source?._tag === 'Ready') URL.revokeObjectURL(source.url)
    this.sources.delete(id)
  }
  private set(id: string, source: Source) {
    this.sources.set(id, source)
    this.emit()
  }
  private emit() { for (const listener of this.listeners) listener() }
}

interface ImageThreadValue {
  readonly images: readonly TranscriptImage[]
  readonly byAnchor: ReadonlyMap<string, readonly TranscriptImage[]>
  readonly store: ImageStore
  readonly open: (id: string) => void
}
const ImageThreadContext = React.createContext<ImageThreadValue | undefined>(undefined)
const noImages: readonly TranscriptImage[] = []
/** Images attached to these transcript items, in thread order. */
export function useAnchorImages(anchors: readonly string[]): readonly TranscriptImage[] {
  const byAnchor = React.useContext(ImageThreadContext)?.byAnchor
  const key = JSON.stringify(anchors)
  return React.useMemo(() => byAnchor === undefined ? noImages : (JSON.parse(key) as string[]).flatMap(anchor => byAnchor.get(anchor) ?? []), [byAnchor, key])
}

/** Owns the thread's image set and its one viewer: prev/next walks every image, in the host's transcript order. */
export function ImageThread({ images, children }: { readonly images: readonly TranscriptImage[]; readonly children: React.ReactNode }) {
  const [store] = React.useState(() => new ImageStore())
  React.useEffect(() => {
    store.retain()
    return () => store.release()
  }, [store])
  const [openId, setOpenId] = React.useState<string>()
  const root = React.useRef<HTMLDivElement>(null)
  const byAnchor = React.useMemo(() => {
    const map = new Map<string, TranscriptImage[]>()
    for (const image of images) map.set(image.anchor, [...map.get(image.anchor) ?? [], image])
    return map
  }, [images])
  const value = React.useMemo<ImageThreadValue>(() => ({ images, byAnchor, store, open: setOpenId }), [images, byAnchor, store])
  const showInTranscript = React.useCallback((image: TranscriptImage) => {
    setOpenId(undefined)
    // The dialog restores focus to its opener on the next frame; the anchor takes focus after that.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      const target = root.current?.querySelector<HTMLElement>(`[data-image-anchor~=${JSON.stringify(image.anchor)}]`)
      if (target === null || target === undefined) return
      target.scrollIntoView({ block: 'center' })
      target.focus({ preventScroll: true })
    }))
  }, [])
  return <ImageThreadContext.Provider value={value}><div ref={root} {...stylex.props(styles.contents)}>
    {children}
    {images.length > 0 && <ThemePortal><ImageViewer images={images} openId={openId} onOpenIdChange={setOpenId} onShowInTranscript={showInTranscript} /></ThemePortal>}
  </div></ImageThreadContext.Provider>
}

function useSource(image: TranscriptImage, active: boolean): Source {
  const store = React.useContext(ImageThreadContext)?.store
  const subscribe = React.useCallback((listener: () => void) => store?.subscribe(listener) ?? (() => {}), [store])
  const source = React.useSyncExternalStore(subscribe, () => store?.get(image.id), () => undefined)
  React.useEffect(() => {
    if (active) store?.request(image)
  }, [active, image, store, source])
  return source ?? loading
}
/** Starts a load once the element is within one viewport of the visible area. */
function useNearViewport<T extends Element>(): [React.RefObject<T | null>, boolean] {
  const ref = React.useRef<T>(null)
  const [near, setNear] = React.useState(() => typeof IntersectionObserver === 'undefined')
  React.useEffect(() => {
    const element = ref.current
    if (near || element === null) return
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) setNear(true)
    }, { rootMargin: '100% 0px' })
    observer.observe(element)
    return () => observer.disconnect()
  }, [near])
  return [ref, near]
}

export const formatImageBytes = (bytes: number): string => bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1).replace(/\.0$/, '')} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`
/** Natural size and byte size, as known; absent facts drop out rather than being invented. */
export const imageFacts = (image: TranscriptImage): string => [image.width !== undefined && image.height !== undefined ? `${image.width}×${image.height}` : undefined, image.bytes === undefined ? undefined : formatImageBytes(image.bytes)].filter(value => value !== undefined).join(' · ')
const ratioOf = (image: TranscriptImage) => image.width !== undefined && image.height !== undefined && image.width > 0 && image.height > 0 ? image.width / image.height : 4 / 3

type Placement = 'input' | 'opened' | 'prose'
const singleBox: Record<Placement, readonly [string, string]> = { input: [g.imageInputW, g.imageInputH], opened: [g.imageOpenedW, g.imageOpenedH], prose: [g.lane, g.imageProseH] }

/** Status text for an image the page is not showing. Never an empty box. */
function ImageStatus({ image, source, onRetry, compact = false }: { readonly image: TranscriptImage; readonly source: Exclude<Source, { _tag: 'Ready' | 'Loading' }>; readonly onRetry: () => void; readonly compact?: boolean }) {
  const text = source._tag === 'Failed' ? 'Couldn’t load image' : source._tag === 'Gone' ? 'Image no longer available · transcript changed' : source._tag === 'TooLarge' ? `Image is ${formatImageBytes(source.bytes)}` : 'This image type can’t be shown'
  return <div role="group" aria-label={`${image.caption.title}: ${text}`} data-image-state={source._tag.toLowerCase()} {...stylex.props(styles.status, compact && styles.statusCompact)}>
    <Icon name="alert" size={14} />
    <span {...stylex.props(styles.statusText)}>{text}</span>
    {source._tag === 'Failed' && <Button onPress={onRetry} aria-label={`Retry loading ${image.caption.title}`} {...stylex.props(styles.textAction)}>Retry</Button>}
    {(source._tag === 'TooLarge' || source._tag === 'Unsupported') && image.onDownload !== undefined && <Button onPress={image.onDownload} aria-label={`Download ${image.caption.title}`} {...stylex.props(styles.textAction)}>Download</Button>}
  </div>
}

/** One image at its reserved size; a press opens the thread viewer at this image. */
function ImageTile({ image, placement, tile }: { readonly image: TranscriptImage; readonly placement: Placement; readonly tile: boolean }) {
  const thread = React.useContext(ImageThreadContext)
  const [ref, near] = useNearViewport<HTMLDivElement>()
  const source = useSource(image, near)
  const ratio = ratioOf(image)
  const [maxW, maxH] = tile ? placement === 'input' ? [g.imageTileW, g.imageTileH] as const : [`calc((${g.imageOpenedW} - ${s.md}) / 2)`, `calc(${g.imageOpenedH} / 2)`] as const : singleBox[placement]
  const natural = image.width === undefined ? maxW : `${image.width}px`
  const box = tile ? styles.tileBox(maxW, maxH) : styles.fitBox(String(ratio), natural, maxW, `calc(${maxH} * ${ratio})`)
  return <div ref={ref} data-testid="transcript-image" data-image-id={image.id} data-image-state={source._tag.toLowerCase()} aria-busy={source._tag === 'Loading' || undefined} {...stylex.props(styles.frame, box)}>
    {source._tag === 'Ready' || source._tag === 'Loading'
      ? <Button aria-label={`View image: ${image.caption.title}`} onPress={() => thread?.open(image.id)} {...stylex.props(styles.imageButton)}>
          {source._tag === 'Ready' ? <img src={source.url} alt="" draggable={false} onError={() => thread?.store.fail(image.id)} {...stylex.props(styles.image, tile && styles.cover)} /> : <span aria-hidden="true" data-testid="image-skeleton" {...stylex.props(styles.skeleton)} />}
        </Button>
      : <ImageStatus image={image} source={source} compact={tile} onRetry={() => thread?.store.retry(image)} />}
  </div>
}

/** Images attached to one transcript item: one image fits its placement box, 2+ become cover tiles. */
export function TranscriptImages({ anchor, placement }: { readonly anchor: string; readonly placement: Placement }) {
  const images = useAnchorImages([anchor])
  if (images.length === 0) return null
  const tile = images.length > 1
  return <div role="group" aria-label={images.length === 1 ? 'Image' : `${images.length} images`} tabIndex={-1} data-image-anchor={anchor} data-image-placement={placement} {...stylex.props(styles.group, tile && placement !== 'input' && styles.twoColumns)}>
    {images.map(image => <ImageTile key={image.id} image={image} placement={placement} tile={tile} />)}
    {placement === 'prose' && !tile && <p data-testid="image-caption" {...stylex.props(styles.proseCaption)}>{images[0]!.caption.title}</p>}
  </div>
}

function RowThumb({ image, interactive }: { readonly image: TranscriptImage; readonly interactive: boolean }) {
  const thread = React.useContext(ImageThreadContext)
  const [ref, near] = useNearViewport<HTMLSpanElement>()
  const source = useSource(image, near)
  const content = source._tag === 'Ready' ? <img src={source.url} alt="" draggable={false} onError={() => thread?.store.fail(image.id)} {...stylex.props(styles.image, styles.cover)} /> : source._tag === 'Loading' ? <span {...stylex.props(styles.skeleton)} /> : <Icon name="alert" size={12} />
  return <span ref={ref} data-testid="image-row-thumb" data-image-id={image.id} data-image-state={source._tag.toLowerCase()} {...stylex.props(styles.thumb)}>
    {interactive ? <Button aria-label={`View image: ${image.caption.title}`} onPress={() => thread?.open(image.id)} {...stylex.props(styles.imageButton, styles.thumbButton)}>{content}</Button> : <span aria-hidden="true" {...stylex.props(styles.thumbInner)}>{content}</span>}
  </span>
}
/** Row excerpt: one image states its own facts, several state their count. */
export const imageExcerpt = (images: readonly TranscriptImage[]): string => images.length === 1 ? imageFacts(images[0]!) || '1 image' : `${images.length} images`
/**
 * 22×16 thumbs after a work row's subject (up to three) plus its excerpt. Inside a row button they are
 * decoration; in the folded summary they are the screenshots' own buttons into the viewer.
 */
export function ImageRowThumbs({ anchors, interactive, excerpt }: { readonly anchors: readonly string[]; readonly interactive: boolean; readonly excerpt: boolean }) {
  const images = useAnchorImages(anchors)
  if (images.length === 0) return null
  return <span data-testid="image-row-thumbs" data-image-anchor={interactive ? anchors.join(' ') : undefined} tabIndex={interactive ? -1 : undefined} {...stylex.props(styles.rowThumbs)}>
    {images.slice(0, 3).map(image => <RowThumb key={image.id} image={image} interactive={interactive} />)}
    {excerpt && <span data-testid="image-row-excerpt" {...stylex.props(styles.excerpt)}>{imageExcerpt(images)}</span>}
  </span>
}

const zoomStep = 1.25, minZoom = 0.25, maxZoom = 8
type Zoom = 'fit' | number
const clampZoom = (value: number) => Math.min(maxZoom, Math.max(minZoom, value))
const extension: Record<TranscriptImageMediaType, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' }
const fileName = (image: TranscriptImage) => `${image.caption.title.replace(/[^\w.-]+/g, '-').replace(/^-+|-+$/g, '') || 'image'}${image.mediaType === undefined || /\.\w+$/.test(image.caption.title) ? '' : `.${extension[image.mediaType]}`}`

/**
 * Full-viewport lightbox over every image in the thread: caption, counter, zoom, prev/next and a
 * filmstrip. ←/→ step, Home/End jump, +/− zoom, 0 fits, 1 shows 100%, Esc closes.
 */
export function ImageViewer({ images, openId, onOpenIdChange, onShowInTranscript }: {
  readonly images: readonly TranscriptImage[]
  readonly openId: string | undefined
  readonly onOpenIdChange: (id: string | undefined) => void
  readonly onShowInTranscript?: (image: TranscriptImage) => void
}) {
  const lastIndex = React.useRef(0)
  const found = openId === undefined ? -1 : images.findIndex(image => image.id === openId)
  // A removed image keeps the viewer at its former position rather than closing it.
  const index = found === -1 ? Math.min(lastIndex.current, images.length - 1) : found
  if (found !== -1) lastIndex.current = found
  const isOpen = openId !== undefined && images.length > 0
  return <ModalOverlay isOpen={isOpen} onOpenChange={open => { if (!open) onOpenIdChange(undefined) }} isKeyboardDismissDisabled={false} {...stylex.props(styles.overlay)}>
    <Modal {...stylex.props(styles.modal)}>
      {isOpen && <ViewerDialog images={images} index={index} onIndex={next => onOpenIdChange(images[next]!.id)} onClose={() => onOpenIdChange(undefined)} onShowInTranscript={onShowInTranscript} />}
    </Modal>
  </ModalOverlay>
}

function ViewerDialog({ images, index, onIndex, onClose, onShowInTranscript }: { readonly images: readonly TranscriptImage[]; readonly index: number; readonly onIndex: (index: number) => void; readonly onClose: () => void; readonly onShowInTranscript?: (image: TranscriptImage) => void }) {
  const store = React.useContext(ImageThreadContext)?.store
  const image = images[index]!
  const source = useSource(image, true)
  const [zoom, setZoom] = React.useState<Zoom>('fit')
  const [natural, setNatural] = React.useState<{ readonly w: number; readonly h: number }>()
  const stage = React.useRef<HTMLDivElement>(null)
  const dialog = React.useRef<HTMLElement>(null)
  const closeButton = React.useRef<HTMLButtonElement>(null)
  const pending = React.useRef<{ readonly x: number; readonly y: number; readonly px: number; readonly py: number }>(undefined)
  const drag = React.useRef<{ readonly x: number; readonly y: number; readonly left: number; readonly top: number }>(undefined)
  const counter = `${index + 1} of ${images.length}`
  React.useEffect(() => { setZoom('fit'); setNatural(undefined) }, [image.id])
  // Neighbours load ahead so a step shows the next image without a skeleton when possible.
  React.useEffect(() => {
    for (const neighbour of [images[index - 1], images[index + 1]]) if (neighbour !== undefined) store?.request(neighbour)
  }, [images, index, store])
  // A step can remove the focused edge button; focus stays inside the dialog.
  React.useEffect(() => {
    const active = document.activeElement
    if (dialog.current !== null && (active === null || active === document.body || !dialog.current.contains(active))) closeButton.current?.focus()
  }, [index])
  const fitScale = React.useCallback(() => {
    const element = stage.current
    if (element === null || natural === undefined) return 1
    return Math.min(1, element.clientWidth / natural.w, element.clientHeight / natural.h)
  }, [natural])
  const scale = zoom === 'fit' ? fitScale() : zoom
  const zoomTo = React.useCallback((next: (current: number) => Zoom, around?: { readonly clientX: number; readonly clientY: number }) => {
    const element = stage.current
    if (natural === undefined) return
    setZoom(current => {
      const from = current === 'fit' ? fitScale() : current
      const to = next(from)
      if (element !== null && around !== undefined && to !== 'fit') {
        const rect = element.getBoundingClientRect()
        const x = around.clientX - rect.left, y = around.clientY - rect.top
        pending.current = { x, y, px: (element.scrollLeft + x) / from, py: (element.scrollTop + y) / from }
      }
      return to
    })
  }, [fitScale, natural])
  React.useLayoutEffect(() => {
    const element = stage.current, anchor = pending.current
    if (element === null || anchor === undefined || zoom === 'fit') return
    pending.current = undefined
    element.scrollLeft = anchor.px * zoom - anchor.x
    element.scrollTop = anchor.py * zoom - anchor.y
  }, [zoom])
  React.useEffect(() => {
    const element = stage.current
    if (element === null) return
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return
      event.preventDefault()
      zoomTo(current => clampZoom(current * (event.deltaY < 0 ? zoomStep : 1 / zoomStep)), event)
    }
    element.addEventListener('wheel', wheel, { passive: false })
    return () => element.removeEventListener('wheel', wheel)
  }, [zoomTo])
  const step = (next: number) => { if (next >= 0 && next < images.length && next !== index) onIndex(next) }
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.altKey || event.metaKey || event.ctrlKey) return
    const inFilmstrip = (event.target as Element).closest('[role="listbox"]') !== null
    const key = event.key
    if (inFilmstrip && (key === 'ArrowLeft' || key === 'ArrowRight' || key === 'Home' || key === 'End')) return
    if (key === 'ArrowLeft') step(index - 1)
    else if (key === 'ArrowRight') step(index + 1)
    else if (key === 'Home') step(0)
    else if (key === 'End') step(images.length - 1)
    else if (key === '+' || key === '=') zoomTo(current => clampZoom(current * zoomStep))
    else if (key === '-' || key === '_') zoomTo(current => clampZoom(current / zoomStep))
    else if (key === '0') setZoom('fit')
    else if (key === '1') zoomTo(() => 1)
    else return
    event.preventDefault()
  }
  const zoomed = zoom !== 'fit' && natural !== undefined && (natural.w * zoom > (stage.current?.clientWidth ?? 0) || natural.h * zoom > (stage.current?.clientHeight ?? 0))
  const caption = <div data-testid="viewer-caption" {...stylex.props(styles.caption)}>
    <strong {...stylex.props(styles.captionTitle)}>{image.caption.title}</strong>
    {image.caption.meta.length > 0 && <span {...stylex.props(styles.captionMeta)}>{image.caption.meta.join(' · ')}</span>}
  </div>
  const navButton = (direction: 'previous' | 'next') => <TooltipTrigger delay={350}>
    <Button aria-label={direction === 'previous' ? 'Previous image' : 'Next image'} data-testid={`viewer-${direction}`} onPress={() => step(direction === 'previous' ? index - 1 : index + 1)} {...stylex.props(styles.nav, direction === 'previous' ? styles.navPrevious : styles.navNext)}>
      <span {...stylex.props(direction === 'previous' && styles.mirrored)}><Icon name="chevron-right" size={18} /></span>
    </Button>
    <Tooltip placement="top" offset={6} {...stylex.props(styles.tooltip)}>{direction === 'previous' ? 'Previous image · ←' : 'Next image · →'}</Tooltip>
  </TooltipTrigger>
  return <Dialog ref={dialog} aria-label={`Image ${counter}: ${image.caption.title}`} data-testid="image-viewer" {...stylex.props(styles.dialog)}>
    <div onKeyDown={onKeyDown} {...stylex.props(styles.shell)}>
      <div data-testid="viewer-bar" {...stylex.props(styles.bar)}>
        <div {...stylex.props(styles.barCaption)}>{caption}</div>
        <span data-testid="viewer-counter" {...stylex.props(styles.counter)}>{counter}</span>
        <div {...stylex.props(styles.controls)}>
          <div data-testid="viewer-zoom" {...stylex.props(styles.controlGroup)}>
            <Button onPress={() => zoomTo(current => clampZoom(current / zoomStep))} isDisabled={natural === undefined || scale <= minZoom} {...stylex.props(styles.textAction)}>Zoom out</Button>
            <Button data-testid="viewer-zoom-readout" onPress={() => zoom === 'fit' ? zoomTo(() => 1) : setZoom('fit')} isDisabled={natural === undefined} {...stylex.props(styles.textAction, styles.readout)}><VisuallyHidden>Zoom: </VisuallyHidden>{zoom === 'fit' ? 'Fit' : `${Math.round(zoom * 100)}%`}</Button>
            <Button onPress={() => zoomTo(current => clampZoom(current * zoomStep))} isDisabled={natural === undefined || scale >= maxZoom} {...stylex.props(styles.textAction)}>Zoom in</Button>
            {source._tag === 'Ready' && <a href={source.url} download={fileName(image)} {...stylex.props(styles.textAction, styles.link)}>Download</a>}
            {onShowInTranscript !== undefined && <Button onPress={() => onShowInTranscript(image)} {...stylex.props(styles.textAction)}>Show in transcript</Button>}
          </div>
          <TooltipTrigger delay={350}>
            <Button ref={closeButton} autoFocus aria-label="Close image viewer" data-testid="viewer-close" onPress={onClose} {...stylex.props(styles.iconButton)}><Icon name="x" size={16} /></Button>
            <Tooltip placement="bottom" offset={6} {...stylex.props(styles.tooltip)}>Close · Esc</Tooltip>
          </TooltipTrigger>
        </div>
      </div>
      <div data-testid="viewer-stage-frame" {...stylex.props(styles.stageFrame)}>
        <div
          ref={stage}
          data-testid="viewer-stage"
          data-zoom={zoom === 'fit' ? 'fit' : String(zoom)}
          onDoubleClick={event => zoom === 'fit' ? zoomTo(() => 1, event) : setZoom('fit')}
          onPointerDown={event => {
            if (event.button !== 0 || (event.target as Element).closest('button') !== null) return
            drag.current = { x: event.clientX, y: event.clientY, left: event.currentTarget.scrollLeft, top: event.currentTarget.scrollTop }
          }}
          onPointerMove={event => {
            const start = drag.current
            if (start === undefined || !zoomed) return
            event.currentTarget.scrollLeft = start.left - (event.clientX - start.x)
            event.currentTarget.scrollTop = start.top - (event.clientY - start.y)
          }}
          onPointerUp={event => {
            const start = drag.current
            drag.current = undefined
            if (start === undefined || zoomed) return
            const dx = event.clientX - start.x, dy = event.clientY - start.y
            if (Math.abs(dx) > 48 && Math.abs(dx) > Math.abs(dy) * 1.5) step(dx < 0 ? index + 1 : index - 1)
          }}
          onPointerCancel={() => { drag.current = undefined }}
          {...stylex.props(styles.stage, zoomed && styles.stagePannable)}
        >
          <div {...stylex.props(styles.stageContent)}>
            {source._tag === 'Ready'
              ? <img key={image.id} data-testid="viewer-image" src={source.url} alt={image.caption.title} draggable={false} onLoad={event => setNatural({ w: event.currentTarget.naturalWidth, h: event.currentTarget.naturalHeight })} onError={() => store?.fail(image.id)} {...stylex.props(zoom === 'fit' || natural === undefined ? styles.fitImage : styles.scaledImage(`${natural.w * zoom}px`, `${natural.h * zoom}px`))} />
              : source._tag === 'Loading'
                ? <span role="status" aria-label={`Loading ${image.caption.title}`} data-testid="viewer-loading" {...stylex.props(styles.viewerSkeleton(String(ratioOf(image))))} />
                : <ImageStatus image={image} source={source} onRetry={() => store?.retry(image)} />}
          </div>
        </div>
        {index > 0 && navButton('previous')}
        {index < images.length - 1 && navButton('next')}
      </div>
      <div {...stylex.props(styles.narrowCaption)}>{caption}</div>
      <ListBox
        aria-label="Images in this thread"
        data-testid="viewer-filmstrip"
        orientation="horizontal"
        selectionMode="single"
        selectionBehavior="replace"
        disallowEmptySelection
        selectedKeys={[image.id]}
        onSelectionChange={(keys: Selection) => {
          if (keys === 'all') return
          const id = [...keys][0]
          const next = images.findIndex(candidate => candidate.id === id)
          if (next !== -1) step(next)
        }}
        {...stylex.props(styles.filmstrip)}
      >
        {images.map((candidate, position) => <ListBoxItem key={candidate.id} id={candidate.id} textValue={candidate.caption.title} aria-label={`${position + 1}. ${candidate.caption.title}`} {...stylex.props(styles.filmItem, position > 0 && images[position - 1]!.anchor !== candidate.anchor && styles.filmDivider)}>
          <FilmThumb image={candidate} selected={position === index} />
        </ListBoxItem>)}
      </ListBox>
      <VisuallyHidden><span role="status" aria-live="polite" data-testid="viewer-announcement">{`Image ${counter}: ${image.caption.title}`}</span></VisuallyHidden>
    </div>
  </Dialog>
}

function FilmThumb({ image, selected }: { readonly image: TranscriptImage; readonly selected: boolean }) {
  const [ref, near] = useNearViewport<HTMLSpanElement>()
  const source = useSource(image, near)
  React.useEffect(() => {
    if (selected) ref.current?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [selected, ref])
  return <span ref={ref} data-image-state={source._tag.toLowerCase()} {...stylex.props(styles.filmThumb, selected && styles.filmSelected)}>
    {source._tag === 'Ready' ? <img src={source.url} alt="" draggable={false} {...stylex.props(styles.image, styles.cover)} /> : source._tag === 'Loading' ? <span {...stylex.props(styles.skeleton)} /> : <Icon name="alert" size={14} />}
  </span>
}

const styles = stylex.create({
  contents: { display: 'contents' },
  group: { display: 'flex', flexWrap: 'wrap', gap: s.sm, minWidth: 0, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  twoColumns: { display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', maxWidth: g.imageOpenedW },
  frame: { position: 'relative', boxSizing: 'border-box', minWidth: 0, maxWidth: '100%', borderRadius: r.md, overflow: 'hidden', backgroundColor: surface.rowActive, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border },
  fitBox: (ratio: string, natural: string, maxW: string, maxHW: string) => ({ aspectRatio: ratio, width: `min(100%, ${natural}, ${maxW}, ${maxHW})` }),
  tileBox: (width: string, height: string) => ({ width, height }),
  imageButton: { display: 'block', width: '100%', height: '100%', padding: 0, margin: 0, borderWidth: 0, backgroundColor: surface.transparent, cursor: 'zoom-in', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  image: { display: 'block', width: '100%', height: '100%', objectFit: 'contain' },
  cover: { objectFit: 'cover' },
  skeleton: { display: 'block', width: '100%', height: '100%', backgroundColor: surface.rowActive },
  status: { boxSizing: 'border-box', display: 'flex', flexWrap: 'wrap', alignItems: 'center', alignContent: 'center', justifyContent: 'center', gap: s.xs, width: '100%', height: '100%', minHeight: g.imageTileH, padding: s.md, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'center', backgroundColor: surface.raised },
  statusCompact: { padding: s.xs, gap: s.xs2 },
  statusText: { minWidth: 0, overflowWrap: 'anywhere' },
  textAction: { minHeight: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', whiteSpace: 'nowrap', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset }, ':disabled': { color: ink.fgMuted, cursor: 'default' } },
  link: { display: 'inline-flex', alignItems: 'center', textDecoration: 'none' },
  proseCaption: { margin: 0, flexBasis: '100%', fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  rowThumbs: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, minWidth: 0, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  thumb: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', width: g.imageThumbW, height: g.imageThumbH, flexShrink: 0, borderRadius: r.keyboard, overflow: 'hidden', backgroundColor: surface.rowActive, color: ink.fgMuted },
  thumbInner: { display: 'block', width: '100%', height: '100%' },
  thumbButton: { cursor: 'pointer' },
  excerpt: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap' },
  overlay: { position: 'fixed', inset: 0, zIndex: 30, backgroundColor: surface.canvas },
  modal: { position: 'absolute', inset: 0, outline: 'none' },
  dialog: { width: '100%', height: '100%', outline: 'none', color: ink.fg, fontFamily: t.fontSans },
  shell: { display: 'grid', gridTemplateRows: { default: `${g.band} minmax(0, 1fr) auto ${g.viewerFilmstrip}`, '@media (max-width: 639px)': `${g.band} minmax(0, 1fr) auto` }, width: '100%', height: '100%' },
  bar: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto minmax(0, 1fr)', alignItems: 'center', gap: s.md, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  barCaption: { minWidth: 0, display: { default: 'block', '@media (max-width: 639px)': 'none' } },
  caption: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  captionTitle: { fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium, color: ink.fg, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  captionMeta: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  counter: { gridColumn: 2, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgSoft, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  controls: { gridColumn: 3, display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: s.xs, minWidth: 0 },
  controlGroup: { display: { default: 'flex', '@media (max-width: 639px)': 'none' }, alignItems: 'center', gap: s.xs2 },
  readout: { minWidth: '6ch', fontVariantNumeric: 'tabular-nums' },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlLg, height: g.controlLg, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  stageFrame: { position: 'relative', minHeight: 0, minWidth: 0 },
  stage: { position: 'absolute', inset: 0, overflow: 'auto', overscrollBehavior: 'contain', touchAction: 'pinch-zoom' },
  stagePannable: { cursor: 'grab', touchAction: 'none' },
  stageContent: { boxSizing: 'border-box', display: 'grid', placeItems: 'center', minWidth: '100%', minHeight: '100%', width: 'max-content', padding: s.xl },
  // The narrow caption row is shorter than the filmstrip it replaces, so one bound serves both layouts.
  fitImage: { display: 'block', maxWidth: `calc(100vw - 2 * ${s.xl})`, maxHeight: `calc(100dvh - ${g.band} - ${g.viewerFilmstrip} - 2 * ${s.xl})`, objectFit: 'contain', userSelect: 'none' },
  scaledImage: (width: string, height: string) => ({ display: 'block', width, height, maxWidth: 'none', userSelect: 'none' }),
  viewerSkeleton: (ratio: string) => ({ display: 'block', aspectRatio: ratio, width: `min(${g.imageOpenedW}, 80vw)`, borderRadius: r.md, backgroundColor: surface.rowActive }),
  nav: { position: 'absolute', top: '50%', transform: 'translateY(-50%)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.viewerNav, height: g.viewerNav, padding: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.full, backgroundColor: surface.raised, color: ink.fg, cursor: 'pointer', ':hover': { backgroundColor: surface.rowActive }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  navPrevious: { left: s.lg }, navNext: { right: s.lg },
  mirrored: { display: 'inline-flex', transform: 'scaleX(-1)' },
  narrowCaption: { display: { default: 'none', '@media (max-width: 639px)': 'block' }, paddingInline: s.lg, paddingBlock: s.md, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  filmstrip: { display: { default: 'flex', '@media (max-width: 639px)': 'none' }, alignItems: 'center', gap: s.sm, paddingInline: s.lg, overflowX: 'auto', overflowY: 'hidden', borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, outline: 'none' },
  filmItem: { flexShrink: 0, borderRadius: r.sm, cursor: 'pointer', outline: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  filmDivider: { marginInlineStart: s.md, paddingInlineStart: s.md, borderInlineStartWidth: g.hairline, borderInlineStartStyle: 'solid', borderInlineStartColor: border.borderStrong, borderRadius: 0 },
  filmThumb: { display: 'flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', width: g.viewerFilmTile, height: `calc(${g.viewerFilmTile} * 3 / 4)`, borderRadius: r.sm, overflow: 'hidden', borderWidth: g.focusRing, borderStyle: 'solid', borderColor: surface.transparent, backgroundColor: surface.rowActive, color: ink.fgMuted },
  filmSelected: { borderColor: ink.fg },
  tooltip: { paddingInline: s.md, paddingBlock: s.xs, borderRadius: r.sm, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
})
