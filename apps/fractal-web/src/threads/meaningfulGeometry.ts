export interface ReadableRect {
  readonly left: number
  readonly right: number
  readonly top: number
  readonly bottom: number
}

/** doc/threads-delivery/adopted-ux-20261116/unread-implementation,
 * “Deterministic visibility default for implementation/review”.
 * Full width and the final min(bodyHeight, two rendered lines) must be exposed.
 */
export const meaningfulBodyRegion = (body: ReadableRect, lineHeight: number, clips: readonly ReadableRect[]): ReadableRect | undefined => {
  const height = body.bottom - body.top
  if (body.right <= body.left || height <= 0 || !Number.isFinite(lineHeight) || lineHeight <= 0) return undefined
  // DOMRect coordinates are prototype accessors, so spreading it drops them.
  const region: ReadableRect = {
    left: body.left,
    right: body.right,
    top: body.bottom - Math.min(height, 2 * lineHeight),
    bottom: body.bottom,
  }
  return clips.every(clip => clip.left <= region.left && clip.right >= region.right && clip.top <= region.top && clip.bottom >= region.bottom)
    ? region : undefined
}

export const overlap = (a: ReadableRect, b: ReadableRect): ReadableRect | undefined => {
  const rect = { left: Math.max(a.left, b.left), right: Math.min(a.right, b.right), top: Math.max(a.top, b.top), bottom: Math.min(a.bottom, b.bottom) }
  return rect.right > rect.left && rect.bottom > rect.top ? rect : undefined
}
