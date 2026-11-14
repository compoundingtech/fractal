type Color = readonly [number, number, number, number]

/** Normalize computed CSS colors (including color-mix and lab) through the browser's sRGB canvas. */
function color(value: string): Color {
  const context = document.createElement('canvas').getContext('2d')!
  context.clearRect(0, 0, 1, 1)
  context.fillStyle = value
  context.fillRect(0, 0, 1, 1)
  const pixel = context.getImageData(0, 0, 1, 1).data
  return [pixel[0]! / 255, pixel[1]! / 255, pixel[2]! / 255, pixel[3]! / 255]
}
function over(foreground: Color, background: Color): Color {
  const alpha = foreground[3] + background[3] * (1 - foreground[3])
  return [0, 1, 2].map(index => (foreground[index]! * foreground[3] + background[index]! * background[3] * (1 - foreground[3])) / alpha).concat(alpha) as unknown as Color
}
function luminance(rgb: Color): number {
  const linear = rgb.slice(0, 3).map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4)
  return linear[0]! * 0.2126 + linear[1]! * 0.7152 + linear[2]! * 0.0722
}
function contrast(element: Element) {
  const backdrop = background(element)
  const ink = over(color(getComputedStyle(element).color), backdrop)
  const [lighter, darker] = [luminance(ink), luminance(backdrop)].sort((a, b) => b - a)
  return (lighter! + 0.05) / (darker! + 0.05)
}
/** Background through shadow boundaries: an element's own paint over its ancestors' (the host included). */
function background(element: Element | null): Color {
  if (element === null) return [1, 1, 1, 1]
  const own = color(getComputedStyle(element).backgroundColor)
  if (own[3] === 1) return own
  const parent = element.parentElement ?? ((element.parentNode as ShadowRoot | null)?.host ?? null)
  return over(own, background(parent))
}

/** Pierre renders into a shadow root; find the first DiffView's rendered lines. */
export function diffShadow(canvas: HTMLElement): ShadowRoot {
  const host = [...canvas.querySelectorAll('[data-diff-file] *')].find(element => element.shadowRoot?.querySelector('[data-content]'))
  if (!host) throw new Error('No rendered DiffView lines')
  return host.shadowRoot!
}

/**
 * The diff tint contract on the kit DiffView (@pierre/diffs): changed-line syntax ink and line
 * numbers read at 4.5:1 or better over their wash, the addition wash is tinted rather than grey, and the
 * one addition variable drives it (an unrelated variable does not).
 */
export function assertDiffTint(canvas: HTMLElement) {
  const root = diffShadow(canvas)
  const view = canvas.querySelector<HTMLElement>('[data-testid="diff-view"], [data-testid="diff-code"]')!
  const added = root.querySelector<HTMLElement>('[data-content] [data-line-type="change-addition"]')
  const removed = root.querySelector<HTMLElement>('[data-content] [data-line-type="change-deletion"]')
  if (!added || !removed) throw new Error('Diff tint requires an added and a removed row')
  const tokens = [added, removed].flatMap(row => [...row.querySelectorAll<HTMLElement>('span')].filter(token => token.children.length === 0 && (token.textContent ?? '').trim() !== ''))
  const numbers = [...root.querySelectorAll<HTMLElement>('[data-gutter] [data-line-type^="change"] [data-line-number-content]')]
  const minimumTextContrast = Math.min(...tokens.map(contrast))
  const minimumGutterContrast = Math.min(...numbers.map(contrast))
  const wash = background(added)
  if (Math.max(...wash.slice(0, 3)) - Math.min(...wash.slice(0, 3)) < 0.01) throw new Error('Added-row wash is neutral grey')
  const follows = (property: string) => {
    const before = getComputedStyle(added).backgroundColor
    view.style.setProperty(property, 'rgb(1, 200, 3)')
    const after = getComputedStyle(added).backgroundColor
    view.style.removeProperty(property)
    return before !== after
  }
  if (!follows('--fractal-diff-addition')) throw new Error('Added wash does not follow the addition variable')
  if (follows('--fractal-diff-unrelated')) throw new Error('Diff tint probe is not specific')
  return { minimumTextContrast: Math.round(minimumTextContrast * 100) / 100, minimumGutterContrast: Math.round(minimumGutterContrast * 100) / 100 }
}
