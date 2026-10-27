import { Schema } from 'effect'

/**
 * Fractal changelog contract. Authored entries live in `.changelog/YYYY-MM-DD-slug.ts`; identity and
 * date derive only from that immutable filename. The manifest is compiled before bundling and carries
 * the exact, opaque build machine version; it is never reconstructed from Git or compared by version.
 */
export const changelogProduct = 'fractal-web'
export const changelogSchemaVersion = 1

/**
 * Each deployment channel compiles exactly one audience; the seen cursor is keyed by channel.
 * Registries default public builds to `main` and private builds to `live`; a stable build selects
 * `stable` through the build plugin, never by comparing versions.
 */
export const changelogAudienceByDeployment = { main: 'public', stable: 'public', live: 'private' } as const
export type ChangelogDeployment = keyof typeof changelogAudienceByDeployment
export type ChangelogAudience = (typeof changelogAudienceByDeployment)[ChangelogDeployment]
export type ChangelogVisibility = 'public' | 'private'

const Line = Schema.String.check(Schema.isMinLength(1), Schema.isTrimmed(), Schema.isPattern(/^[^\r\n]*$/u))
const Title = Line.check(Schema.isMaxLength(100))
const Summary = Line.check(Schema.makeFilter((text: string) => {
  const sentences = text.match(/[.!?](?=\s|$)/gu)?.length ?? 0
  return /[.!?]$/u.test(text) && sentences >= 1 && sentences <= 3 ? true : 'expected 1-3 complete sentences'
}))
const Kind = Schema.Literals(['feature', 'improvement', 'fix'])
/** Only links to confirmed public pull requests; never commits of a private or unmerged stack. */
const PublicPrUrl = Schema.String.check(Schema.isPattern(/^https:\/\/github\.com\/[A-Za-z0-9-]+\/[A-Za-z0-9._-]+\/pull\/[1-9]\d*$/u))
const Visibility = Schema.Literals(['public', 'private'])

const isCalendarDate = (date: string): boolean => {
  const [year, month, day] = date.split('-').map(Number)
  if (year === undefined || month === undefined || day === undefined) return false
  const parsed = new Date(Date.UTC(year, month - 1, day))
  return parsed.getUTCFullYear() === year && parsed.getUTCMonth() === month - 1 && parsed.getUTCDate() === day
}
const EntryDate = Schema.String.check(Schema.isPattern(/^\d{4}-\d{2}-\d{2}$/u), Schema.makeFilter((date: string) => isCalendarDate(date) ? true : 'invalid calendar date'))
const entryIdPattern = /^(\d{4}-\d{2}-\d{2})-[a-z0-9]+(?:-[a-z0-9]+)*$/u
const EntryId = Schema.String.check(Schema.isPattern(entryIdPattern), Schema.makeFilter((id: string) => isCalendarDate(id.slice(0, 10)) ? true : 'invalid calendar date'))

/** Authored payload: the default export of one `.changelog/YYYY-MM-DD-slug.ts` file. */
export const ChangelogEntrySourceSchema = Schema.Struct({
  title: Title,
  summary: Summary,
  kind: Schema.optionalKey(Kind),
  publicPrUrl: Schema.optionalKey(PublicPrUrl),
})
export type ChangelogEntrySource = typeof ChangelogEntrySourceSchema.Type

export const ChangelogEntrySchema = Schema.Struct({
  id: EntryId,
  date: EntryDate,
  visibility: Visibility,
  title: Title,
  summary: Summary,
  kind: Schema.optionalKey(Kind),
  publicPrUrl: Schema.optionalKey(PublicPrUrl),
})
export type ChangelogEntry = typeof ChangelogEntrySchema.Type

export const ChangelogManifestSchema = Schema.Struct({
  schemaVersion: Schema.Literal(changelogSchemaVersion),
  product: Schema.Literal(changelogProduct),
  build: Schema.Struct({ machineVersion: Line }),
  entries: Schema.Array(ChangelogEntrySchema),
})
export type ChangelogManifest = typeof ChangelogManifestSchema.Type

/** One registry row. `file` is the bare filename inside its `.changelog/` directory. */
export interface ChangelogSourceRegistration {
  readonly file: string
  readonly source: unknown
  readonly visibility: ChangelogVisibility
}

export type ChangelogErrorReason =
  | 'invalid-source'
  | 'invalid-filename'
  | 'invalid-manifest'
  | 'duplicate-registration'
  | 'duplicate-entry-id'
  | 'inventory-mismatch'
  | 'unsorted-entries'
  | 'build-mismatch'
  | 'private-entry-in-public-audience'

export class ChangelogError extends Error {
  readonly reason: ChangelogErrorReason
  constructor(reason: ChangelogErrorReason, message: string, options?: ErrorOptions) {
    super(`changelog ${reason}: ${message}`, options)
    this.name = 'ChangelogError'
    this.reason = reason
  }
}

const strict = { onExcessProperty: 'error', errors: 'all' } as const
const decodeSource = Schema.decodeUnknownSync(ChangelogEntrySourceSchema, strict)
const decodeManifestShape = Schema.decodeUnknownSync(ChangelogManifestSchema, strict)

/** Strict authored-source decoder; unknown fields are rejected rather than stripped. */
export const decodeChangelogEntrySource = (input: unknown, file: string): ChangelogEntrySource => {
  try {
    return decodeSource(input)
  } catch (cause) {
    throw new ChangelogError('invalid-source', `${file}: ${String(cause)}`, { cause })
  }
}

/** `YYYY-MM-DD-slug.ts` → `{id, date}`; the filename is the only source of identity. */
export const changelogEntryIdentity = (file: string): { readonly id: string; readonly date: string } => {
  const id = file.endsWith('.ts') ? file.slice(0, -'.ts'.length) : ''
  const date = entryIdPattern.exec(id)?.[1]
  if (date === undefined || !isCalendarDate(date)) throw new ChangelogError('invalid-filename', `entries are named YYYY-MM-DD-slug.ts with a real date: ${file}`)
  return { id, date }
}

/** Newest first: the date prefix makes descending id order chronological and deterministic. */
export const compareChangelogEntryIds = (left: string, right: string): number => left < right ? 1 : left > right ? -1 : 0

export const duplicateValues = (values: readonly string[]): readonly string[] => {
  const seen = new Set<string>()
  const duplicates = new Set<string>()
  for (const value of values) {
    if (seen.has(value)) duplicates.add(value)
    seen.add(value)
  }
  return [...duplicates].sort()
}

/**
 * Strict browser and build boundary: unknown fields, duplicate ids, impossible dates, ids that
 * disagree with their date, unsorted entries, and a manifest for another build are all rejected.
 */
export const decodeChangelogManifest = (input: unknown, expected?: { readonly machineVersion: string }): ChangelogManifest => {
  let manifest: ChangelogManifest
  try {
    manifest = decodeManifestShape(input)
  } catch (cause) {
    throw new ChangelogError('invalid-manifest', String(cause), { cause })
  }
  const ids = manifest.entries.map(({ id }) => id)
  const duplicates = duplicateValues(ids)
  if (duplicates.length > 0) throw new ChangelogError('duplicate-entry-id', duplicates.join(', '))
  for (const entry of manifest.entries)
    if (!entry.id.startsWith(`${entry.date}-`)) throw new ChangelogError('invalid-manifest', `entry ${entry.id} disagrees with its date ${entry.date}`)
  for (let index = 1; index < ids.length; index++)
    if (compareChangelogEntryIds(ids[index - 1]!, ids[index]!) >= 0) throw new ChangelogError('unsorted-entries', `${ids[index - 1]} must follow ${ids[index]}`)
  if (expected !== undefined && manifest.build.machineVersion !== expected.machineVersion)
    throw new ChangelogError('build-mismatch', `manifest ${manifest.build.machineVersion} does not describe build ${expected.machineVersion}`)
  return manifest
}
