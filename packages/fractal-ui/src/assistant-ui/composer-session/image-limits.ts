/**
 * Composer image limits and the user-facing copy for every way a staged image can be refused.
 * Hosts validate with `composerImageRejection` before staging and throw its message; the composer
 * announces adapter messages verbatim and maps runtime-level refusals with `attachmentAddErrorNotice`.
 */
export const composerImageTypes = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] as const

export interface ComposerImageLimits {
  readonly maxCount: number
  readonly maxBytes: number
  readonly types: readonly string[]
}
export const composerImageLimits: ComposerImageLimits = { maxCount: 4, maxBytes: 10 * 1024 * 1024, types: composerImageTypes }

export type ComposerImageRejection =
  | { readonly _tag: 'Type'; readonly name: string; readonly type: string }
  | { readonly _tag: 'Size'; readonly name: string; readonly bytes: number }
  | { readonly _tag: 'Count'; readonly name: string }

/** First limit the file breaks, given how many images are already staged; `undefined` when it fits. */
export const composerImageRejection = (file: { readonly name: string; readonly type: string; readonly size: number }, staged: number, limits: ComposerImageLimits = composerImageLimits): ComposerImageRejection | undefined =>
  !limits.types.includes(file.type) ? { _tag: 'Type', name: file.name, type: file.type }
  : file.size > limits.maxBytes ? { _tag: 'Size', name: file.name, bytes: file.size }
  : staged >= limits.maxCount ? { _tag: 'Count', name: file.name }
  : undefined

const typeLabels: Readonly<Record<string, string>> = { 'image/png': 'PNG', 'image/jpeg': 'JPEG', 'image/gif': 'GIF', 'image/webp': 'WebP', 'image/svg+xml': 'SVG', 'image/*': 'any image' }
/** Human list of accepted MIME types: "PNG, JPEG, GIF or WebP". */
const acceptedTypes = (types: readonly string[]) => {
  const labels = types.map(type => typeLabels[type] ?? type)
  return labels.length < 2 ? labels.join('') : `${labels.slice(0, -1).join(', ')} or ${labels.at(-1)}`
}

/** Binary megabytes, matching how the byte limit is computed: 10 × 1024² reads as "10 MB". */
export const formatImageBytes = (bytes: number) => {
  const megabytes = bytes / (1024 * 1024)
  return `${Number.isInteger(megabytes) ? megabytes : megabytes.toFixed(1)} MB`
}

export const composerImageRejectionMessage = (rejection: ComposerImageRejection, limits: ComposerImageLimits = composerImageLimits): string => {
  switch (rejection._tag) {
    case 'Type': return `${rejection.name} isn't a supported image. Use ${acceptedTypes(limits.types)}.`
    case 'Size': return `${rejection.name} is over the ${formatImageBytes(limits.maxBytes)} image limit.`
    case 'Count': return `${rejection.name} wasn't added. A message can include up to ${limits.maxCount} images.`
  }
}

export interface AttachmentAddError {
  readonly reason: 'not-accepted' | 'adapter-error' | 'no-adapter'
  readonly message: string
  readonly contentType?: string
}

/**
 * Copy for one refused add. Adapter messages are already user copy; the runtime refuses
 * unaccepted types before the adapter sees the file, so only the content type is known there.
 */
export const attachmentAddErrorNotice = (error: AttachmentAddError, accept: string): string => {
  switch (error.reason) {
    case 'adapter-error': return error.message
    case 'no-adapter': return 'Images can\'t be attached here.'
    case 'not-accepted': {
      const subject = error.contentType === undefined || error.contentType === '' ? 'This file type' : `${typeLabels[error.contentType] ?? error.contentType} files`
      const types = accept === '*' ? [] : accept.split(',').map(type => type.trim()).filter(type => type !== '')
      return types.length === 0 ? `${subject} can't be attached.` : `${subject} can't be attached. Use ${acceptedTypes(types)}.`
    }
  }
}
