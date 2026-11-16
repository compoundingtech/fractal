/**
 * Composer send contract. The host owns identities, upload state, retry policy, sessions and
 * transport; the kit only renders these facts and reports intent through callbacks.
 */
export type ComposerImageMediaType = 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp'

export type ComposerImage = {
  /** Runtime attachment identity, never a manufactured native blob ID. */
  readonly id: string
  readonly name: string
  readonly mediaType: ComposerImageMediaType
  readonly size: number
  readonly previewUrl?: string
  readonly state: { readonly _tag: 'Ready' }
    | { readonly _tag: 'Uploading'; readonly progress?: number }
    | { readonly _tag: 'Failed'; readonly detail: string }
}

export type ComposerRecipientReadout = {
  /** Canonical recipient ref; exposed to assistive technology and the readout detail. */
  readonly ref: string
  readonly label: string
  /** Authoritative active session for a plain continuation. */
  readonly sessionId?: string
  /** The active session changed while this draft was open. */
  readonly sessionChanged?: boolean
}

export type ComposerAvailability =
  | { readonly _tag: 'Ready' }
  | { readonly _tag: 'Offline'; readonly detail: string; readonly onReconnect?: () => void }
  | { readonly _tag: 'MissingGrant'; readonly detail: string; readonly onRecover?: () => void }
  | { readonly _tag: 'Revoked'; readonly detail: string; readonly code?: string; readonly onRecover?: () => void }
  | { readonly _tag: 'Unavailable'; readonly detail: string }

/** Reply context for one authoritative incoming mail row; `messageId` is its message resource ID. */
export type ComposerReply = {
  readonly messageId: string
  readonly sender: string
  readonly subject?: string
  readonly excerpt?: string
}

/** Image limits the host's attachments adapter declares; the kit pre-check, picker filter and copy follow them. */
export interface ComposerImagePolicy {
  readonly maxCount: number
  readonly maxBytes: number
  readonly mediaTypes: readonly ComposerImageMediaType[]
}

/** Default when the host declares nothing: four PNG, JPEG, GIF or WebP images per message, each up to 10 MiB. */
export const composerImagePolicy: ComposerImagePolicy = {
  maxCount: 4,
  maxBytes: 10 * 1024 * 1024,
  mediaTypes: ['image/png', 'image/jpeg', 'image/gif', 'image/webp'],
}

const mediaTypeNames: Record<ComposerImageMediaType, string> = { 'image/png': 'PNG', 'image/jpeg': 'JPEG', 'image/gif': 'GIF', 'image/webp': 'WebP' }

export const isComposerImageType = (type: string): type is ComposerImageMediaType => (composerImagePolicy.mediaTypes as readonly string[]).includes(type)

export function formatImageSize(bytes: number): string {
  return bytes >= 1024 * 1024 ? `${(bytes / (1024 * 1024)).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`
}

/** A declared limit reads as written: 10 MiB is "10 MB", not "10.0 MB". */
export function formatImageLimit(bytes: number): string {
  const mebibytes = bytes / (1024 * 1024)
  return Number.isInteger(mebibytes) ? `${mebibytes} MB` : formatImageSize(bytes)
}

/** "PNG, JPEG, GIF or WebP" for the declared types, in declared order. */
export function describeImageTypes(policy: ComposerImagePolicy): string {
  const names = policy.mediaTypes.map(type => mediaTypeNames[type])
  return names.length <= 1 ? names.join('') : `${names.slice(0, -1).join(', ')} or ${names[names.length - 1]}`
}

export const describeImageCount = (count: number) => `${count} ${count === 1 ? 'image' : 'images'}`

/** Files the runtime may receive, in order, and one plain sentence for every refusal. */
export function screenComposerImages(files: readonly File[], retained: number, policy: ComposerImagePolicy = composerImagePolicy): { readonly accepted: readonly File[]; readonly refused: string | undefined } {
  const accepted: File[] = []
  const reasons: string[] = []
  for (const file of files) {
    const name = file.name === '' ? 'That file' : file.name
    if (!(policy.mediaTypes as readonly string[]).includes(file.type)) { reasons.push(`${name} ${file.type === '' ? 'has no file type' : `is ${file.type}`}; Fractal takes ${describeImageTypes(policy)}.`); continue }
    if (file.size > policy.maxBytes) { reasons.push(`${name} is ${formatImageSize(file.size)}; Fractal takes images up to ${formatImageLimit(policy.maxBytes)}.`); continue }
    if (retained + accepted.length >= policy.maxCount) { reasons.push(`A message carries at most ${describeImageCount(policy.maxCount)}.`); continue }
    accepted.push(file)
  }
  return { accepted, refused: reasons.length === 0 ? undefined : [...new Set(reasons)].join(' ') }
}

/** Why submit waits on the controlled image tray, if it does. */
export function imageSubmitBlock(images: readonly ComposerImage[] | undefined): string | undefined {
  if (images === undefined) return undefined
  const failed = images.filter(image => image.state._tag === 'Failed').length
  if (failed > 0) return failed === 1 ? 'Retry or remove the image that did not upload to send.' : `Retry or remove the ${failed} images that did not upload to send.`
  const uploading = images.filter(image => image.state._tag === 'Uploading').length
  if (uploading > 0) return uploading === 1 ? 'Waiting for the image to finish uploading.' : `Waiting for ${uploading} images to finish uploading.`
  return undefined
}
