import type { Arrangement } from '../../clients/typescript/fractal-client';

export type NodeRef = { _tag: 'Folder'; id: string } | { _tag: 'Subject'; subject: string };
export type Parent = NodeRef | { _tag: 'Root' };
export type Intent = { node: NodeRef; parent: Parent; before: NodeRef | null };
export type MembershipInput = { member: string; container: string; position: { bucket: string | null; key: string }; revision: string };
export type ProjectionError = { _tag: 'NotReady' | 'ArrangementUnavailable' | 'InvalidResolution' } | { _tag: 'Unsupported'; required: number; advertised: number };
export type ProjectedNode = { node: NodeRef; parent: Parent; shownUnder: Parent; key?: string; revision?: string; source: 'person' | 'fallback'; children: NodeRef[] };
export type Tree = { roots: NodeRef[]; nodes: Record<string, ProjectedNode>; arrangementRevision: string; deletedFolders: string[] };
export const nodeKey = (node: NodeRef): string => node._tag === 'Folder' ? `folder:${node.id}` : `subject:${node.subject}`;
export const sameNode = (a: Parent, b: Parent): boolean => a._tag === 'Root' ? b._tag === 'Root' : b._tag !== 'Root' && nodeKey(a) === nodeKey(b);
const parent = (id: string | null): Parent => id === null ? { _tag: 'Root' } : { _tag: 'Folder', id };
// Rust strings compare Unicode scalar values, not JavaScript UTF-16 code units.
const ordinal = (a: string, b: string): number => {
  const left = a[Symbol.iterator](), right = b[Symbol.iterator]();
  for (;;) {
    const x = left.next(), y = right.next();
    if (x.done || y.done) return Number(y.done) - Number(x.done);
    const difference = x.value.codePointAt(0)! - y.value.codePointAt(0)!;
    if (difference) return difference;
  }
};

/** Complete authoritative projection only; inventory augments v1, never v2 membership. */
export const buildTree = (arrangement: Arrangement, memberships?: readonly MembershipInput[], inventory: readonly string[] = []): Tree | { error: ProjectionError } => {
  const fail = (_tag: 'NotReady' | 'ArrangementUnavailable' | 'InvalidResolution'): { error: ProjectionError } => ({ error: { _tag } });
  if (arrangement.deleted) return fail('ArrangementUnavailable');
  const version = arrangement.body.version, required = memberships === undefined ? 1 : 2;
  if (version !== required) return { error: { _tag: 'Unsupported', required, advertised: version } };
  const resolved = arrangement.resolved;
  if (!resolved) return fail('NotReady');
  const tree: Tree = { roots: [], nodes: {}, arrangementRevision: arrangement.revision, deletedFolders: [] };
  let incomplete = false;
  const insert = (node: NodeRef, raw: Parent, shownUnder: Parent, key?: string, revision?: string): void => {
    tree.nodes[nodeKey(node)] = { node, parent: raw, shownUnder, ...(key === undefined ? {} : { key, revision }), source: key === undefined ? 'fallback' : 'person', children: [] };
  };
  for (const id of Object.keys(arrangement.body.folders).sort(ordinal)) {
    const folder = arrangement.body.folders[id];
    if (folder.tombstone != null) { tree.deletedFolders.push(id); continue; }
    if (!Object.hasOwn(resolved.parents, id)) { incomplete = true; continue; }
    insert({ _tag: 'Folder', id }, parent(folder.position.value.parent), parent(resolved.parents[id]), folder.position.value.key, folder.position.revision);
  }
  const unavailable = new Set<string>();
  if (arrangement.body.version === 1) for (const [subject, register] of Object.entries(arrangement.body.placements)) {
    if (!Object.hasOwn(resolved.folders, subject)) { incomplete = true; unavailable.add(subject); continue; }
    insert({ _tag: 'Subject', subject }, parent(register.value.folder), parent(resolved.folders[subject]), register.value.key, register.revision);
  }
  for (const member of memberships ?? []) {
    const node: NodeRef = { _tag: 'Subject', subject: member.member };
    if (member.container !== arrangement.id || Object.hasOwn(tree.nodes, nodeKey(node))) return fail('InvalidResolution');
    insert(node, parent(member.position.bucket), parent(member.position.bucket), member.position.key, member.revision);
  }
  if (version === 1) for (const subject of inventory) {
    const node: NodeRef = { _tag: 'Subject', subject };
    if (!unavailable.has(subject) && !Object.hasOwn(tree.nodes, nodeKey(node))) insert(node, parent(null), parent(null));
  }
  // Missing resolved entries take precedence over ancestry errors, but not invalid memberships.
  if (incomplete) return fail('NotReady');
  const states = new Map<string, boolean | null>();
  for (const key of Object.keys(tree.nodes)) {
    const path: string[] = [];
    let cursor = key, valid = false;
    for (;;) {
      if (states.has(cursor)) { valid = states.get(cursor) === true; break; }
      states.set(cursor, null); path.push(cursor);
      const shown = tree.nodes[cursor].shownUnder;
      if (shown._tag === 'Root') { valid = true; break; }
      if (shown._tag !== 'Folder' || !Object.hasOwn(tree.nodes, nodeKey(shown))) break;
      cursor = nodeKey(shown);
    }
    for (const item of path) states.set(item, valid);
    if (!valid) return fail('InvalidResolution');
  }
  const rank = (node: ProjectedNode): number => node.node._tag === 'Folder' ? 0 : node.key === undefined ? 2 : 1;
  const ordered = Object.values(tree.nodes).sort((a, b) => rank(a) - rank(b) || ordinal(a.key ?? '', b.key ?? '') || ordinal(nodeKey(a.node), nodeKey(b.node)));
  for (const item of ordered) {
    if (item.shownUnder._tag === 'Root') tree.roots.push(item.node);
    else tree.nodes[nodeKey(item.shownUnder)].children.push(item.node);
  }
  return tree;
};

const digits = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
const integerLength = (key: string): number => key[0] >= 'a' ? key.charCodeAt(0) - 95 : 92 - key.charCodeAt(0);
const validKey = (key: string): boolean => /^[A-Za-z][0-9A-Za-z]+$/.test(key) && key.length <= 128 && key.length >= integerLength(key)
  && (key.length === integerLength(key) || !key.endsWith('0')) && !(key[0] === 'A' && /^0+$/.test(key.slice(1, integerLength(key))));

/** Exact canonical trie capacity and evenly spaced ranks at the minimum sufficient horizon. */
export const spread = (lower: string | null, upper: string | null, count: number): string[] | undefined => {
  if ((lower !== null && !validKey(lower)) || (upper !== null && !validKey(upper)) || (lower !== null && upper !== null && lower >= upper)
    || !Number.isSafeInteger(count) || count < 0 || count > 0xffffffff) return undefined;
  if (count === 0) return [];
  const powers = [1n];
  for (let i = 1; i <= 128; i++) powers.push(powers[i - 1] * 62n);
  for (let horizon = 2; horizon <= 128; horizon++) {
    const cache = new Map<string, bigint>();
    const capacity = (prefix: string): bigint => {
      if ((upper !== null && prefix >= upper) || (lower !== null && prefix < lower && !lower.startsWith(prefix))) return 0n;
      const integer = prefix.length === 0 ? 0 : integerLength(prefix);
      if (integer > horizon) return 0n;
      const zero = prefix.startsWith('A') && /^0*$/.test(prefix.slice(1, Math.min(prefix.length, integer)));
      if (zero && prefix.length >= integer) return 0n;
      const after = lower === null || prefix > lower, valid = validKey(prefix);
      if (prefix.length > 0 && after && (upper === null || !upper.startsWith(prefix))) {
        return powers[horizon - prefix.length] - (zero ? powers[horizon - integer] : prefix.length >= integer && !valid ? 1n : 0n);
      }
      const cached = cache.get(prefix);
      if (cached !== undefined) return cached;
      let total = valid && after ? 1n : 0n;
      if (prefix.length < horizon) for (const char of prefix === '' ? digits.slice(10) : digits) total += capacity(prefix + char);
      cache.set(prefix, total);
      return total;
    };
    const total = capacity('');
    if (total < BigInt(count)) continue;
    const select = (prefix: string, rank: bigint): string => {
      if (validKey(prefix) && (lower === null || prefix > lower)) {
        if (rank === 0n) return prefix;
        rank--;
      }
      for (const char of prefix === '' ? digits.slice(10) : digits) {
        const available = capacity(prefix + char);
        if (rank < available) return select(prefix + char, rank);
        rank -= available;
      }
      throw new Error('Canonical rank exceeded admitted capacity');
    };
    return Array.from({ length: count }, (_, index) => select('', total * BigInt(index + 1) / (BigInt(count) + 1n)));
  }
  return undefined;
};
