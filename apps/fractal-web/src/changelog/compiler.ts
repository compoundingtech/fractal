import {
  ChangelogError,
  changelogEntryIdentity,
  changelogProduct,
  changelogSchemaVersion,
  compareChangelogEntryIds,
  decodeChangelogEntrySource,
  decodeChangelogManifest,
  duplicateValues,
  type ChangelogAudience,
  type ChangelogEntry,
  type ChangelogManifest,
  type ChangelogSourceRegistration,
  type ChangelogVisibility,
} from './contract.ts'

/** A file found in a `.changelog/` directory; build tooling lists it, browsers never do. */
export interface ChangelogInventoryFile {
  readonly file: string
  readonly visibility: ChangelogVisibility
}

/**
 * Validates and orders entries for one audience. A public audience rejects any private registration
 * instead of filtering it, so a public manifest can never be derived from private sources. With an
 * inventory, registered files and directory files must be a bijection per visibility.
 */
export const compileChangelogEntries = ({ sources, audience, inventory }: {
  readonly sources: readonly ChangelogSourceRegistration[]
  readonly audience: ChangelogAudience
  readonly inventory?: readonly ChangelogInventoryFile[]
}): readonly ChangelogEntry[] => {
  if (audience === 'public') {
    const leaked = sources.filter(({ visibility }) => visibility !== 'public').map(({ file }) => file)
    if (leaked.length > 0) throw new ChangelogError('private-entry-in-public-audience', leaked.join(', '))
  }
  const keys = sources.map(({ file, visibility }) => `${visibility}/${file}`)
  const duplicates = duplicateValues(keys)
  if (duplicates.length > 0) throw new ChangelogError('duplicate-registration', duplicates.join(', '))
  if (inventory !== undefined) {
    const listed = inventory.map(({ file, visibility }) => `${visibility}/${file}`)
    const registered = new Set(keys)
    const present = new Set(listed)
    const unregistered = listed.filter((key) => !registered.has(key)).toSorted()
    const missing = keys.filter((key) => !present.has(key)).toSorted()
    if (unregistered.length > 0 || missing.length > 0)
      throw new ChangelogError('inventory-mismatch', [
        unregistered.length === 0 ? undefined : `unregistered: ${unregistered.join(', ')}`,
        missing.length === 0 ? undefined : `registered but missing: ${missing.join(', ')}`,
      ].filter((part) => part !== undefined).join('; '))
  }
  const entries = sources.map(({ file, source, visibility }): ChangelogEntry => {
    const { title, summary, kind, publicPrUrl } = decodeChangelogEntrySource(source, file)
    return {
      ...changelogEntryIdentity(file),
      visibility,
      title,
      summary,
      ...(kind === undefined ? {} : { kind }),
      ...(publicPrUrl === undefined ? {} : { publicPrUrl }),
    }
  })
  const duplicateIds = duplicateValues(entries.map(({ id }) => id))
  if (duplicateIds.length > 0) throw new ChangelogError('duplicate-entry-id', duplicateIds.join(', '))
  return entries.toSorted((left, right) => compareChangelogEntryIds(left.id, right.id))
}

/** Deterministic manifest for one build: same sources and machine version, same value and bytes. */
export const compileChangelog = ({ build, sources, audience, inventory }: {
  readonly build: { readonly machineVersion: string }
  readonly sources: readonly ChangelogSourceRegistration[]
  readonly audience: ChangelogAudience
  readonly inventory?: readonly ChangelogInventoryFile[]
}): ChangelogManifest => decodeChangelogManifest({
  schemaVersion: changelogSchemaVersion,
  product: changelogProduct,
  build: { machineVersion: build.machineVersion },
  entries: compileChangelogEntries({ sources, audience, ...(inventory === undefined ? {} : { inventory }) }),
}, build)
