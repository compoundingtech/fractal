import { ClientError, type ActionOf, type Arrangement, type OrderedMembership } from '@fractal/client'
import type { MembershipEditEvent } from './edit.ts'
import type { MembershipDelivery, SidebarJournalRequest, SidebarJournalState } from './sidebarJournal.ts'
import type { MembershipComplete } from './memberships.ts'
import { generateKeyBetween, generateNKeysBetween } from 'fractional-indexing'
import { drawnParents, insertAt, isLive, liveAncestor, positionKey, project, wouldCycle, type FolderDoc, type Insertion, type Projection, type ProjectedFolder } from './core.mts'
import { refusalText, sidebarOperations, type ArrangementEditorState, type RestageVerdict, type SidebarOperation } from './edit.ts'
import type { SidebarAgent, SidebarCreateFolder, SidebarDropVerdict, SidebarMove, SidebarNode, SidebarRowStatus } from './sidebarContract.ts'

/** A seat's row key is not its shared membership subject. Lifecycle does not filter this roster. */
export interface SidebarSeat { readonly id: string; readonly subject: string; readonly host: string; readonly label: string; readonly project?: string; readonly children?: readonly SidebarAgent[] }
export type SidebarDocument = FolderDoc & { readonly roster: readonly SidebarSeat[] }
export type SidebarCollapse = ReadonlyMap<string, boolean>
const compare = (left: string, right: string): number => {
  const a = Array.from(left, (value) => value.codePointAt(0)!)
  const b = Array.from(right, (value) => value.codePointAt(0)!)
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i]! - b[i]!
  return a.length - b.length
}

export const sidebarTree = (projection: Projection, roster: readonly SidebarSeat[], collapse: SidebarCollapse): SidebarNode[] => {
  const agent = (seat: SidebarSeat): SidebarNode => ({
    _tag: 'Agent', id: seat.id, subject: seat.subject, label: seat.label,
    // A seat with further missions starts open, so they show; otherwise children wait behind the chevron.
    children: seat.children, collapsed: collapse.get(seat.id) ?? !seat.children?.some(child => child.missionId !== undefined),
  })
  const seats = new Map<string, SidebarNode[]>()
  for (const seat of roster) {
    const rows = seats.get(seat.subject) ?? []
    rows.push(agent(seat))
    seats.set(seat.subject, rows)
  }
  const folder = (item: ProjectedFolder): SidebarNode => ({
    _tag: 'Folder', id: item.id, label: item.name, collapsed: collapse.get(item.id) ?? false,
    children: [...item.folders.map(folder), ...item.members.flatMap((subject) => seats.get(subject) ?? [])],
  })
  const unfiled = new Set(projection.unfiled)
  const rows = roster.filter(seat => unfiled.has(seat.subject)).toSorted((a, b) => compare(a.host, b.host) || compare(a.id, b.id))
  const projects = new Map<string, typeof rows>()
  for (const row of rows) {
    const project = row.project ?? ''
    const members = projects.get(project) ?? []
    members.push(row)
    projects.set(project, members)
  }
  return [...projection.folders.map(folder), ...[...projects].map(([project, members]): SidebarNode => {
    const id = project === '' ? 'wf/unfiled' : `wf/project/${encodeURIComponent(project)}`
    return {
      _tag: 'Group', id, label: project === '' ? 'Unfiled' : project, collapsed: collapse.get(id) ?? false,
      children: members.map(agent),
    }
  })]
}

/** Filtering is a view operation; all move plans continue to use the complete document. */
export const filterSidebarTree = (tree: readonly SidebarNode[], query: string): SidebarNode[] => {
  const needle = query.trim().toLocaleLowerCase()
  if (needle === '') return [...tree]
  return tree.flatMap((node): SidebarNode[] => {
    if (node._tag === 'Agent') {
      const children = node.children === undefined ? undefined : filterSidebarTree(node.children, query) as readonly SidebarAgent[]
      if (node.label.toLocaleLowerCase().includes(needle)) return [{ ...node, collapsed: false }]
      return children?.length ? [{ ...node, collapsed: false, children }] : []
    }
    const children = filterSidebarTree(node.children, query)
    return children.length === 0 ? [] : [{ ...node, collapsed: false, children }]
  })
}

type Item = { readonly kind: 'folder' | 'agent'; readonly id: string; readonly parent: string | null }
const itemFor = (doc: SidebarDocument, id: string): Item | undefined => {
  if (isLive(doc, id)) return { kind: 'folder', id, parent: drawnParents(doc).get(id) ?? null }
  const seat = doc.roster.find((row) => row.id === id)
  return seat === undefined ? undefined : { kind: 'agent', id: seat.subject, parent: liveAncestor(doc, doc.placements[seat.subject]?.folder ?? null) }
}
type Sibling = { readonly id: string; readonly key: string; readonly visible: boolean }
/** The full stored partition: dormant placements (absent from the roster) keep reserved keys but no visible position. */
const siblingsFor = (doc: FolderDoc & { readonly roster?: readonly SidebarSeat[] }, kind: Item['kind'], parent: string | null): Sibling[] => {
  if (kind === 'folder') return [...drawnParents(doc)].filter(([, value]) => value === parent).map(([id]) => ({ id, key: positionKey(doc, id), visible: true })).sort((a, b) => compare(a.key, b.key) || compare(a.id, b.id))
  const live = doc.roster === undefined ? undefined : new Set(doc.roster.map((seat) => seat.subject))
  return Object.entries(doc.placements).filter(([, value]) => liveAncestor(doc, value.folder) === parent)
    .map(([id, value]) => ({ id, key: value.key, visible: live === undefined || live.has(id) })).sort((a, b) => compare(a.key, b.key) || compare(a.id, b.id))
}

/** `siblings` is the full partition without the item; `index` is the key slot, already mapped past dormant keys. */
type MovePlan = { readonly item: Item; readonly parent: string | null; readonly index: number; readonly siblings: readonly Sibling[]; readonly unchanged: boolean }
const planMove = (doc: SidebarDocument, move: SidebarMove): MovePlan | { refused: string } => {
  if (move.items.length !== 1) return { refused: 'Move one item at a time.' }
  const item = itemFor(doc, move.items[0])
  if (item === undefined) return { refused: 'This item is no longer available.' }
  const target = move.target
  let parent: string | null
  let sibling: Item | undefined
  if (target._tag === 'Into') {
    if (!isLive(doc, target.folder)) return { refused: 'This folder is no longer available.' }
    parent = target.folder
  } else if (target._tag === 'Root') {
    if (item.kind !== 'folder') return { refused: 'Unfiled agents use automatic host order.' }
    if (!Number.isInteger(target.index) || target.index < 0) return { refused: 'Choose a valid folder position.' }
    parent = null
  } else if (target._tag === 'Unfiled') {
    if (item.kind !== 'agent') return { refused: 'Folders cannot move into Unfiled.' }
    parent = null
  } else {
    sibling = itemFor(doc, target.sibling)
    if (sibling === undefined) return { refused: 'This item is no longer available.' }
    if (sibling.kind !== item.kind) return { refused: 'Folders and agents have separate ordering.' }
    parent = sibling.parent
    if (item.kind === 'agent' && parent === null) return { refused: 'Unfiled agents use automatic host order.' }
    // A gap inside the consecutive seats of one subject is never a membership slot.
    if (item.kind === 'agent') {
      const rows = doc.roster.filter((seat) => seat.subject === sibling?.id)
      const edge = target._tag === 'Before' ? rows[0] : rows.at(-1)
      if (edge?.id !== target.sibling) return { refused: 'Move between agents, not between seats of one agent.' }
    }
  }
  if (item.kind === 'folder' && wouldCycle(doc, item.id, parent)) return { refused: 'A folder cannot go inside itself or its subfolders.' }
  const all = siblingsFor(doc, item.kind, parent)
  const visible = all.filter((row) => row.visible)
  const others = visible.filter((row) => row.id !== item.id)
  let index = others.length
  if (target._tag === 'Root') {
    if (target.index > visible.length) return { refused: 'Choose a valid folder position.' }
    // Root targets name the gap in the current tree, before removal of an existing root mover.
    const from = visible.findIndex((row) => row.id === item.id)
    index = target.index - (from >= 0 && from < target.index ? 1 : 0)
  } else if (sibling !== undefined && sibling.id !== item.id) {
    index = others.findIndex((row) => row.id === sibling.id) + (target._tag === 'After' ? 1 : 0)
  }
  const unchanged = sibling?.id === item.id || (item.parent === parent && (parent === null && item.kind === 'agent' || visible.findIndex((row) => row.id === item.id) === index))
  const siblings = all.filter((row) => row.id !== item.id)
  const anchor = others[index]
  return { item, parent, siblings, index: anchor === undefined ? siblings.length : siblings.indexOf(anchor), unchanged }
}

/** Keep aligned with MAX_KEY_BYTES and valid_key in crates/fractal-schema/src/arrangements.rs, and with the fractal TUI port in the dotfiles repository's flakes/fractal. */
const maxArrangementKeyBytes = 128
const validArrangementKey = (key: string): boolean => {
  if (!/^[A-Za-z]/.test(key) || /[^0-9A-Za-z]/.test(key) || key.length > maxArrangementKeyBytes) return false
  // Canonical keys are ASCII, so their string length is also their UTF-8 byte length.
  const head = key.charCodeAt(0)
  const integerLength = head >= 97 ? head - 97 + 2 : 90 - head + 2
  return key.length >= integerLength
    && (key.length === integerLength || !key.endsWith('0'))
    && !(key[0] === 'A' && /^0+$/.test(key.slice(1, integerLength)))
}

/**
 * Fractal admits only canonical fractional-indexing keys (generated ArrangementKey), not core.mts's legacy claim alphabet.
 * Invalid or colliding neighbour keys are rekeyed in the same edit; noncanonical or oversized outputs rekey the whole partition.
 */
export const nativeInsertAt = (siblings: readonly string[], index: number): Insertion => {
  try {
    const insertion = insertAt(siblings, index, generateKeyBetween)
    if (validArrangementKey(insertion.key) && insertion.rekeyed.every(([, key]) => validArrangementKey(key))) return insertion
  } catch {
    // Invalid neighbours can leave no local gap; use the same full-partition fallback.
  }
  const keys = generateNKeysBetween(null, null, siblings.length + 1)
  return { key: keys[index]!, rekeyed: siblings.map((_, at): [number, string] => [at, keys[at < index ? at : at + 1]!]) }
}
export const canDrop = (doc: SidebarDocument, move: SidebarMove): SidebarDropVerdict => {
  const plan = planMove(doc, move)
  return 'refused' in plan ? plan : { ok: true }
}
/** Fresh-state verdict: an invalid plan stays refused with fixed copy; an already-satisfied plan writes nothing. */
export const restageMove = (doc: SidebarDocument, move: SidebarMove): RestageVerdict => {
  const plan = planMove(doc, move)
  if ('refused' in plan) return { _tag: 'Refused', sentence: plan.refused }
  if (plan.unchanged) return { _tag: 'Satisfied' }
  if (plan.item.kind === 'agent' && plan.parent === null) return { _tag: 'Operations', operations: [sidebarOperations.placeSubject({ subject: plan.item.id, folder: null, key: 'a0' })] }
  const insertion = nativeInsertAt(plan.siblings.map((row) => row.key), plan.index)
  const operation = (id: string, key: string): SidebarOperation => plan.item.kind === 'folder'
    ? sidebarOperations.moveFolder({ id, parent: plan.parent, key })
    : sidebarOperations.placeSubject({ subject: id, folder: plan.parent, key })
  return { _tag: 'Operations', operations: [...insertion.rekeyed.map(([index, key]) => operation(plan.siblings[index]!.id, key)), operation(plan.item.id, insertion.key)] }
}
export const operationsForMove = (doc: SidebarDocument, move: SidebarMove): SidebarOperation[] => {
  const verdict = restageMove(doc, move)
  return verdict._tag === 'Operations' ? [...verdict.operations] : []
}
/** Agent rows a move names, bound to the subjects they showed when the gesture was accepted. */
export const boundRows = (roster: readonly SidebarSeat[], move: SidebarMove): ReadonlyMap<string, string> => {
  const rows = [move.items[0], ...(move.target._tag === 'Before' || move.target._tag === 'After' ? [move.target.sibling] : [])]
  return new Map(rows.flatMap((id) => {
    const seat = roster.find((row) => row.id === id)
    return seat === undefined ? [] : [[id, seat.subject] as const]
  }))
}
/** A row key rebound to another subject must never redirect a retried move to that subject. */
export const restageBoundMove = (doc: SidebarDocument, move: SidebarMove, bound: ReadonlyMap<string, string>): RestageVerdict => {
  for (const [id, subject] of bound) if (doc.roster.find((row) => row.id === id)?.subject !== subject)
    return { _tag: 'Refused', sentence: 'This agent changed; select it again.' }
  return restageMove(doc, move)
}
export const operationsForCreate = (doc: FolderDoc, intent: SidebarCreateFolder, id: string): SidebarOperation[] => {
  const parent = intent.withAgent !== undefined ? null : intent.parent !== null && isLive(doc, intent.parent) ? intent.parent : null
  const siblings = siblingsFor(doc, 'folder', parent)
  const insertion = nativeInsertAt(siblings.map((row) => row.key), siblings.length)
  return [...insertion.rekeyed.map(([index, key]) => sidebarOperations.moveFolder({ id: siblings[index]!.id, parent, key })),
    sidebarOperations.createFolder({ id, parent, key: insertion.key, name: intent.name.trim() || 'New folder' }),
    ...(intent.withAgent === undefined ? [] : [sidebarOperations.placeSubject({ subject: intent.withAgent, folder: id, key: 'a0' })])]
}
export const restageCreate = (doc: FolderDoc, intent: SidebarCreateFolder, id: string): RestageVerdict =>
  isLive(doc, id) ? { _tag: 'Satisfied' }
  : doc.folders[id] !== undefined ? { _tag: 'Refused', sentence: 'This folder was deleted; create it again.' }
  : { _tag: 'Operations', operations: operationsForCreate(doc, intent, id) }
export const restageRename = (doc: FolderDoc, intent: { readonly id: string; readonly name: string }): RestageVerdict =>
  !isLive(doc, intent.id) ? { _tag: 'Refused', sentence: 'This folder is no longer available.' }
  : doc.folders[intent.id]?.name?.value === (intent.name.trim() || 'New folder') ? { _tag: 'Satisfied' }
  : { _tag: 'Operations', operations: operationsForRename(doc, intent) }
export const restageDelete = (doc: FolderDoc, intent: { readonly id: string }): RestageVerdict =>
  isLive(doc, intent.id) ? { _tag: 'Operations', operations: operationsForDelete(doc, intent) }
  : doc.folders[intent.id]?.deleted !== undefined ? { _tag: 'Satisfied' } : { _tag: 'Refused', sentence: 'This folder is no longer available.' }
export const operationsForRename = (doc: FolderDoc, intent: { readonly id: string; readonly name: string }): SidebarOperation[] => {
  const name = intent.name.trim() || 'New folder'
  return !isLive(doc, intent.id) || doc.folders[intent.id]?.name?.value === name ? [] : [sidebarOperations.renameFolder({ id: intent.id, name })]
}
/** Tombstoning alone lifts contents with their original keys; never rewrite or delete members. */
export const operationsForDelete = (doc: FolderDoc, intent: { readonly id: string }): SidebarOperation[] =>
  isLive(doc, intent.id) ? [sidebarOperations.deleteFolder({ id: intent.id })] : []

export const rowStatus = (editorState: Pick<ArrangementEditorState, 'phase' | 'refusal' | 'retryReady'> & { readonly onRetry?: () => void }, pendingTargets: ReadonlyMap<string, string>): Map<string, SidebarRowStatus> => {
  const result = new Map<string, SidebarRowStatus>()
  for (const [row, target] of pendingTargets) {
    if (editorState.refusal?.targets.includes(target)) result.set(row, {
      _tag: 'Refused', reason: refusalText(editorState.refusal),
      ...(editorState.retryReady && editorState.onRetry !== undefined ? { onRetry: editorState.onRetry } : {}),
    })
    else if (editorState.phase === 'pending') result.set(row, { _tag: 'Pending' })
  }
  return result
}

export const sidebarProjection = (doc: SidebarDocument): Projection => project(doc, new Set(doc.roster.map((seat) => seat.subject)))

/** Exact canonical-key capacity, matching Rust keys/capacity.rs (BigInt ranks). */
export const spreadMembershipKeys = (lower: string | null, upper: string | null, count: number): string[] | undefined => {
  if (!Number.isSafeInteger(count) || count < 0 || count > 4097 || lower !== null && !validArrangementKey(lower) || upper !== null && !validArrangementKey(upper) || lower !== null && upper !== null && lower >= upper) return undefined
  if (count === 0) return []
  const digits = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'
  const powers = [1n]
  for (let i = 1; i <= 128; i++) powers.push(powers[i - 1]! * 62n)
  for (let max = 2; max <= 128; max++) {
    const cache = new Map<string, bigint>()
    const capacity = (prefix: string): bigint => {
      if (upper !== null && prefix >= upper || lower !== null && prefix < lower && !lower.startsWith(prefix)) return 0n
      const head = prefix.charCodeAt(0)
      const integer = prefix === '' ? 0 : head >= 97 ? head - 97 + 2 : 90 - head + 2
      if (integer > max) return 0n
      const zero = prefix.startsWith('A') && /^0*$/.test(prefix.slice(1, integer))
      if (zero && prefix.length >= integer) return 0n
      const after = lower === null || prefix > lower
      const valid = validArrangementKey(prefix)
      if (prefix !== '' && after && (upper === null || !upper.startsWith(prefix))) {
        return powers[max - prefix.length]! - (zero ? powers[max - integer]! : prefix.length >= integer && !valid ? 1n : 0n)
      }
      const cached = cache.get(prefix)
      if (cached !== undefined) return cached
      let total = valid && after ? 1n : 0n
      if (prefix.length < max) for (const char of prefix === '' ? digits.slice(10) : digits) total += capacity(prefix + char)
      cache.set(prefix, total)
      return total
    }
    const total = capacity('')
    if (total < BigInt(count)) continue
    const select = (prefix: string, rank: bigint): string => {
      if (validArrangementKey(prefix) && (lower === null || prefix > lower)) {
        if (rank === 0n) return prefix
        rank--
      }
      for (const char of prefix === '' ? digits.slice(10) : digits) {
        const size = capacity(prefix + char)
        if (rank < size) return select(prefix + char, rank)
        rank -= size
      }
      throw new RangeError('canonical key rank outside admitted capacity')
    }
    return Array.from({ length: count }, (_, i) => select('', total * BigInt(i + 1) / BigInt(count + 1)))
  }
  return undefined
}

export type MembershipIntent = SidebarJournalState['intent']
export type MembershipPlan = { readonly folders: Extract<SidebarOperation, { op: 'folder.move' }>[]; readonly memberships: Extract<SidebarJournalRequest['operations'], { Memberships: unknown }>['Memberships'] }
type PlanNode = { readonly node: MembershipIntent['node']; readonly parent: string | null; readonly shown: string | null; readonly key: string }
export type MembershipPlanResult = { readonly _tag: 'PlannedV2'; readonly edit: MembershipPlan } | { readonly _tag: 'RefusedV2'; readonly refusal: string }
type MembershipRefusal = Extract<MembershipPlanResult, { _tag: 'RefusedV2' }>
export type MembershipProjection = MembershipRefusal | {
  readonly _tag: 'ProjectedV2'; readonly nodes: ReadonlyMap<string, PlanNode>
  readonly partitions: ReadonlyMap<string | null, { readonly folder: readonly PlanNode[]; readonly subject: readonly PlanNode[] }>
}
const membershipNodeId = (node: MembershipIntent['node']): string => node.kind === 'folder' ? `folder:${node.id}` : `subject:${node.subject}`
/** Complete-input projection validates and indexes once. Catalog rows never create members. */
export const projectMemberships = (arrangement: Arrangement, memberships: readonly OrderedMembership[]): MembershipProjection => {
  const refuse = (refusal: string): MembershipRefusal => ({ _tag: 'RefusedV2', refusal })
  if (arrangement.deleted) return refuse('ArrangementUnavailable')
  if (arrangement.body.version !== 2) return refuse('Unsupported')
  const resolved = arrangement.resolved
  if (!resolved) return refuse('NotReady')
  const nodes = new Map<string, PlanNode>()
  const id = membershipNodeId
  let missingParent = false
  for (const [folder, value] of Object.entries(arrangement.body.folders)) {
    if (value.tombstone !== null) continue
    if (!Object.hasOwn(resolved.parents, folder)) { missingParent = true; continue }
    nodes.set(`folder:${folder}`, { node: { kind: 'folder', id: folder }, parent: value.position.value.parent, shown: resolved.parents[folder]!, key: value.position.value.key })
  }
  for (const member of memberships) {
    const node = { kind: 'subject', subject: member.member } as const
    if (member.container !== arrangement.id || nodes.has(id(node))) return refuse('InvalidResolution')
    nodes.set(id(node), { node, parent: member.position.bucket, shown: member.position.bucket, key: member.position.key })
  }
  // Rust validates member identity/container before reporting an incomplete resolved-parent map.
  if (missingParent) return refuse('NotReady')
  for (const row of nodes.values()) {
    const seen = new Set<string>([id(row.node)])
    let parent = row.shown
    while (parent !== null) {
      const ancestor = nodes.get(`folder:${parent}`)
      if (!ancestor || seen.has(id(ancestor.node))) return refuse('InvalidResolution')
      seen.add(id(ancestor.node)); parent = ancestor.shown
    }
  }
  const partitions = new Map<string | null, { folder: PlanNode[]; subject: PlanNode[] }>()
  for (const row of nodes.values()) {
    let partition = partitions.get(row.shown)
    if (!partition) { partition = { folder: [], subject: [] }; partitions.set(row.shown, partition) }
    partition[row.node.kind].push(row)
  }
  for (const partition of partitions.values()) for (const rows of [partition.folder, partition.subject])
    rows.sort((a, b) => compare(a.key, b.key) || compare(id(a.node), id(b.node)))
  return { _tag: 'ProjectedV2', nodes, partitions }
}

/** One retained projection per host. Snapshot store_index is deliberately not a collection revision. */
export class MembershipProjectionCache {
  #held: { id: string; revision: string; scope: MembershipComplete['scope']; frontier: MembershipComplete['frontier']; catalog: string; presentation: string; model: Extract<MembershipProjection, { _tag: 'ProjectedV2' }> } | undefined
  get(arrangement: Arrangement, complete: MembershipComplete, catalog: string, presentation: string): MembershipProjection {
    const held = this.#held
    const scope = complete.scope
    if (scope.owner !== arrangement.owner || scope.container !== arrangement.id) {
      this.#held = undefined
      return { _tag: 'RefusedV2', refusal: 'InvalidResolution' }
    }
    if (arrangement.deleted || arrangement.body.version !== 2 || !arrangement.resolved) {
      this.#held = undefined
      return projectMemberships(arrangement, complete.members)
    }
    // PR21 supplies resolved.parents directly: validate at most 1,024 folders, never scan members on a cache hit.
    let resolutionChanged = false
    for (const folder in arrangement.body.folders) {
      if (arrangement.body.folders[folder]!.tombstone !== null) continue
      if (!Object.hasOwn(arrangement.resolved.parents, folder)) {
        this.#held = undefined
        return projectMemberships(arrangement, complete.members)
      }
      if (held && held.model.nodes.get(`folder:${folder}`)?.shown !== arrangement.resolved.parents[folder]) resolutionChanged = true
    }
    if (held && !resolutionChanged && held.id === arrangement.id && held.revision === arrangement.revision
      && held.catalog === catalog && held.presentation === presentation
      && held.scope.owner === scope.owner && held.scope.container === scope.container && held.scope.endpoint === scope.endpoint
      && held.scope.host === scope.host && held.scope.projection === scope.projection
      && held.frontier.epoch === complete.frontier.epoch && held.frontier.changed_index === complete.frontier.changed_index
      && held.frontier.live_count === complete.frontier.live_count) return held.model
    const model = projectMemberships(arrangement, complete.members)
    // Copy primitive key values; host inputs may be updated in place. Incomplete resolutions are never retained.
    this.#held = model._tag === 'ProjectedV2' ? { id: arrangement.id, revision: arrangement.revision, scope: { ...scope }, frontier: { ...complete.frontier }, catalog, presentation, model } : undefined
    return model
  }
}

/** Per-input planning consumes the cached complete projection, never rebuilds its ancestry or partitions. */
export const planMembershipMove = (model: MembershipProjection, intent: MembershipIntent): MembershipPlanResult => {
  if (model._tag === 'RefusedV2') return model
  const { nodes, partitions } = model
  const id = membershipNodeId
  const refuse = (refusal: string): MembershipRefusal => ({ _tag: 'RefusedV2', refusal })
  const node = nodes.get(id(intent.node))
  if (!node) return refuse('NodeMissing')
  if (intent.node.kind === 'folder' && intent.node.id === intent.parent) return refuse('Self')
  if (intent.parent !== null && !nodes.has(`folder:${intent.parent}`)) return refuse('ParentMissing')
  if (intent.node.kind === 'folder') {
    let parent = intent.parent
    while (parent !== null) {
      if (parent === intent.node.id) return refuse('Cycle')
      parent = nodes.get(`folder:${parent}`)!.shown
    }
  }
  const anchor = intent.before === null ? undefined : nodes.get(id(intent.before))
  if (intent.before !== null && (!anchor || id(intent.before) === id(intent.node) || anchor.node.kind !== node.node.kind || anchor.shown !== intent.parent)) return refuse('AnchorMoved')
  const all = partitions.get(intent.parent)?.[node.node.kind] ?? []
  const from = all.indexOf(node)
  if (node.parent === intent.parent && node.shown === intent.parent && all[from + 1] === anchor) return refuse('NoChange')
  const siblings = all.filter(row => row !== node)
  const index = anchor === undefined ? siblings.length : siblings.indexOf(anchor)
  let best: { key: string; rekeys: [number, string][] } | undefined
  for (let end = index; end <= siblings.length; end++) {
    const keys = spreadMembershipKeys(siblings[index - 1]?.key ?? null, siblings[end]?.key ?? null, end - index + 1)
    if (keys) { best = { key: keys[0]!, rekeys: keys.slice(1).map((key, at) => [index + at, key]) }; break }
  }
  for (let start = index - 1; start >= 0 && index - start < (best?.rekeys.length ?? Infinity); start--) {
    const keys = spreadMembershipKeys(siblings[start - 1]?.key ?? null, siblings[index]?.key ?? null, index - start + 1)
    if (keys) { best = { key: keys.at(-1)!, rekeys: keys.slice(0, -1).map((key, at) => [start + at, key]) }; break }
  }
  if (!best) return refuse('OrderingUnavailable')
  if (best.rekeys.length + 1 > 1024) return refuse('TooManyOperations')
  const changes: [PlanNode, string][] = [[node, best.key], ...best.rekeys.map(([at, key]): [PlanNode, string] => [siblings[at]!, key])]
  const folders: MembershipPlan['folders'] = []
  const members: { op: 'place'; member: string; bucket: string | null; key: string }[] = []
  for (const [row, key] of changes) {
    if (row.node.kind === 'folder') folders.push({ op: 'folder.move', id: row.node.id, parent: intent.parent, key })
    else members.push({ op: 'place', member: row.node.subject, bucket: intent.parent, key })
  }
  if (new TextEncoder().encode(JSON.stringify(folders.length ? folders : members)).length > 1024 * 1024) return refuse('TooManyBytes')
  return { _tag: 'PlannedV2', edit: { folders, memberships: members } }
}

/** Send only requests returned by the journal executor's Send effect. Actor stays transport-owned. */
export const membershipWriteRequest = (request: SidebarJournalRequest): ActionOf<'arrangement.edit'> | ActionOf<'arrangement.membership.edit'> => {
  const { fence, identity, operations } = request
  const common = { api_version: 'st3.client.v0' as const, id: identity.action_id, idempotency_key: identity.idempotency_key,
    fence: { snapshot_id: fence.snapshot.id, subject_revisions: { [fence.scope.container]: fence.revision } } }
  const parameters = { subject: fence.scope.container, owner: fence.scope.owner }
  return 'Folders' in operations
    ? { ...common, type: 'arrangement.edit', parameters: { ...parameters, operations: [...operations.Folders] } }
    : { ...common, type: 'arrangement.membership.edit', parameters: { ...parameters, operations: [...operations.Memberships] } }
}
/** Forward Send delivery unchanged: admission errors cannot disprove an earlier uncertain commit. */
export const membershipWriteResult = (request: SidebarJournalRequest, delivery: MembershipDelivery, result: unknown): MembershipEditEvent => {
  const identity = request.identity
  if (typeof result === 'object' && result !== null && 'action_id' in result && result.action_id === identity.action_id && 'status' in result) {
    if (result.status === 'accepted' || result.status === 'completed') return { _tag: 'Ack', identity }
    if (result.status === 'rejected') return { _tag: 'Refused', identity, refusal: 'Conflict' }
  }
  if (delivery === 'Replay') return { _tag: 'Unknown', identity }
  if (result instanceof ClientError) {
    const { code, details } = result.response
    if (details?.applied === 'unknown') return { _tag: 'Unknown', identity }
    const refusal = code === 'stale-fence' ? 'StaleFence'
      : ['forbidden', 'arrangement-owner-forbidden', 'membership-owner-forbidden'].includes(code) ? 'Unauthorized'
      : ['arrangement-folder-deleted', 'arrangement-retired', 'not-found'].includes(code) ? 'DestinationMissing'
      : code === 'invalid-membership-operations' ? 'InvalidPlan'
      : details?.applied === 'none' ? code === 'idempotency-conflict' ? 'Conflict' : 'InvalidPlan' : undefined
    if (refusal) return { _tag: 'Refused', identity, refusal }
  }
  return { _tag: 'Unknown', identity }
}
