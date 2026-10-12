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

/** st accepts four PNG, JPEG, GIF or WebP images per message, each up to 10 MiB. */
export const composerImagePolicy = {
  maxCount: 4,
  maxBytes: 10 * 1024 * 1024,
  mediaTypes: ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] as const satisfies readonly ComposerImageMediaType[],
} as const

export const isComposerImageType = (type: string): type is ComposerImageMediaType =>
  (composerImagePolicy.mediaTypes as readonly string[]).includes(type)

export function formatImageSize(bytes: number): string {
  return bytes >= 1024 * 1024 ? `${(bytes / (1024 * 1024)).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`
}

/** Files the runtime may receive, in order, and one plain sentence for every refusal. */
export function screenComposerImages(files: readonly File[], retained: number): { readonly accepted: readonly File[]; readonly refused: string | undefined } {
  const accepted: File[] = []
  const reasons: string[] = []
  for (const file of files) {
    const name = file.name === '' ? 'That file' : file.name
    if (!isComposerImageType(file.type)) { reasons.push(`${name} is ${file.type === '' ? 'of an unknown type' : file.type}; st takes PNG, JPEG, GIF or WebP.`); continue }
    if (file.size > composerImagePolicy.maxBytes) { reasons.push(`${name} is ${formatImageSize(file.size)}; st takes images up to 10 MB.`); continue }
    if (retained + accepted.length >= composerImagePolicy.maxCount) { reasons.push(`A message carries at most ${composerImagePolicy.maxCount} images.`); continue }
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
