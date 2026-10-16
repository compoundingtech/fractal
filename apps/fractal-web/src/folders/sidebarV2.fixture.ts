import { Schema } from 'effect'
import { Arrangement } from '@smalltalk/st3-client/schema'
import type { Arrangement as ArrangementModel } from '@smalltalk/st3-client'
import vectors from '../../../../fixtures/clients/sidebar-v2.json'

/** Golden projection IDs are symbolic; the wire schema requires UUIDv7 folder identities. */
export const canonicalSidebarVector = (vector: typeof vectors.cases[number]) => {
  const ids = new Map<string, string>()
  const folderId = (id: string): string => {
    let canonical = ids.get(id)
    if (canonical === undefined) {
      canonical = `00000000-0000-7000-8000-${(ids.size + 1).toString(16).padStart(12, '0')}`
      ids.set(id, canonical)
    }
    return canonical
  }
  const parent = (id: string | null): string | null => id === null ? null : folderId(id)
  const input = vector.arrangement
  const decoded = Schema.decodeUnknownSync(Schema.toEncoded(Arrangement))({ ...input,
    body: { ...input.body,
      folders: Object.fromEntries(Object.entries(input.body.folders).flatMap(([id, folder]) => folder === undefined ? [] : [[folderId(id), {
        ...folder, position: { ...folder.position, value: { ...folder.position.value, parent: parent(folder.position.value.parent) } },
      }]])),
      placements: Object.fromEntries(Object.entries(input.body.placements).flatMap(([id, placement]) => placement === undefined ? [] : [[id, {
        ...placement, value: { ...placement.value, folder: parent(placement.value.folder) },
      }]])),
    },
    resolved: {
      parents: Object.fromEntries(Object.entries(input.resolved.parents).flatMap(([id, value]) => value === undefined ? [] : [[folderId(id), parent(value)]])),
      folders: Object.fromEntries(Object.entries(input.resolved.folders).flatMap(([id, value]) => value === undefined ? [] : [[id, parent(value)]])),
    },
  })
  const arrangement: ArrangementModel = {
    id: decoded.id, kind: decoded.kind, owner: decoded.owner, body: decoded.body,
    deleted: decoded.deleted, revision: decoded.revision, updated_at: decoded.updated_at,
    ...(decoded.resolved == null ? {} : { resolved: decoded.resolved }),
  }
  return { arrangement, folderId }
}
