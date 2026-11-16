import { PINS_ARRANGEMENT_UUID } from '@fractal/client';
import type { OrderedMembership, OrderedMembershipOperation } from '@fractal/client';

export type QuickAccessDestination = { kind: 'agent' | 'mission' | 'document' | 'glass' | 'resource'; subject: string };
export type QuickAccessPresentation = { title: string; detail?: string; availability?: 'available' | 'unavailable' };
export type QuickAccessItem = {
  id: string;
  destination: QuickAccessDestination | null;
  title: string;
  availability: 'available' | 'unavailable' | 'unsupported';
  detail?: string;
};

/** Pins belong to the discovered person, never a session, agent or guessed default owner. */
export function quickAccessSelection(actor: string): { owner: string; subject: string } {
  const owner = /^(person\/[^/\s\p{Cc}]+)(?:\/session\/[^/\s\p{Cc}]+)?$/u.exec(actor)?.[1];
  if (!owner) throw new TypeError('Quick access requires a discovered person principal');
  return { owner, subject: `arrangement/${owner}/${PINS_ARRANGEMENT_UUID}` };
}

/** Stable navigation destinations supported by the clients; sessions and inbox rows are not pins. */
export function quickAccessDestination(subject: string): QuickAccessDestination | null {
  if (new TextEncoder().encode(subject).length > 512 || /[\s\p{Cc}]/u.test(subject)) return null;
  const parts = subject.split('/');
  if (parts.length < 2 || parts.some(part => part.length === 0)) return null;
  switch (parts[0]) {
    case 'agent': case 'mission': case 'glass': case 'resource': return { kind: parts[0], subject };
    // A document shortcut follows its name; immutable @version references are not member subjects.
    case 'doc': return subject.includes('@') ? null : { kind: 'document', subject };
    default: return null;
  }
}

/** Server order and membership are authoritative. Missing local inventory never unpins a row. */
export function quickAccessItems(
  members: readonly OrderedMembership[],
  resolve: (destination: QuickAccessDestination) => QuickAccessPresentation | undefined,
): QuickAccessItem[] {
  return members.map(({ member }) => {
    const destination = quickAccessDestination(member);
    if (!destination) return { id: member, destination: null, title: member, availability: 'unsupported', detail: 'This client cannot open this shortcut.' };
    const presentation = resolve(destination);
    return { id: member, destination, title: presentation?.title || member,
      availability: presentation?.availability ?? (presentation ? 'available' : 'unavailable'),
      ...(presentation?.detail ? { detail: presentation.detail } : presentation ? {} : { detail: 'Details are not loaded on this client.' }) };
  });
}

export type QuickAccessPlacePlan =
  | { operations: OrderedMembershipOperation[]; rebalanced: boolean }
  | { refusal: 'unsupported-destination' | 'invalid-memberships' | 'anchor-missing' | 'ordering-unavailable' | 'too-many-members' };
const digits = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
export const validQuickAccessKey = (key: string): boolean => {
  if (!/^[A-Za-z][0-9A-Za-z]+$/.test(key) || key.length > 128) return false;
  const integerLength = key.charAt(0) >= 'a' ? key.charCodeAt(0) - 95 : 92 - key.charCodeAt(0);
  return key.length >= integerLength && !(key.charAt(0) === 'A' && key.slice(1, integerLength) === '0'.repeat(integerLength - 1))
    && (key.length === integerLength || !key.endsWith('0'));
};

/** Place a stable destination before an observed member (null appends). The host supplies its
 * canonical fractional-key generator. Equal concurrent keys or a bounded exhausted gap are
 * repaired atomically in the current published order, never by sorting names or local inventory. */
export function planQuickAccessPlace(
  members: readonly OrderedMembership[], member: string, before: string | null,
  between: (lower: string | null, upper: string | null) => string | null,
): QuickAccessPlacePlan {
  if (!quickAccessDestination(member)) return { refusal: 'unsupported-destination' };
  if (new Set(members.map(item => item.member)).size !== members.length ||
    members.some((item, at) => item.position.bucket !== null || !validQuickAccessKey(item.position.key)
      || at > 0 && item.position.key < (members[at - 1]?.position.key ?? item.position.key))) return { refusal: 'invalid-memberships' };
  if (before === member) return { operations: [], rebalanced: false };
  const rest = members.filter(item => item.member !== member);
  const index = before === null ? rest.length : rest.findIndex(item => item.member === before);
  if (index < 0) return { refusal: 'anchor-missing' };
  const order = [...rest.slice(0, index).map(item => item.member), member, ...rest.slice(index).map(item => item.member)];
  if (order.length === members.length && order.every((id, at) => id === members[at]?.member)) return { operations: [], rebalanced: false };
  const lower = rest[index - 1]?.position.key ?? null, upper = rest[index]?.position.key ?? null;
  let key: string | null = null;
  if (lower === null || upper === null || lower < upper) {
    try { key = between(lower, upper); } catch { return { refusal: 'ordering-unavailable' }; }
    if (key !== null && (!validQuickAccessKey(key) || lower !== null && key <= lower || upper !== null && key >= upper)) return { refusal: 'ordering-unavailable' };
  }
  if (key !== null) return { operations: [{ op: 'place', member, bucket: null, key }], rebalanced: false };
  if (order.length > 1024) return { refusal: 'too-many-members' };
  return { operations: order.map((id, at) => ({ op: 'place', member: id, bucket: null,
    key: `c${digits.charAt(Math.floor(at / (62 * 62)))}${digits.charAt(Math.floor(at / 62) % 62)}${digits.charAt(at % 62)}` })), rebalanced: true };
}
