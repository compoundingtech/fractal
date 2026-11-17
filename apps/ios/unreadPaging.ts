// Local domain seam for thread.list(marked_unread=true), not a wire response.
// The generated-client adapter translates its page/cursor fields into this port.
export type MarkedUnreadPage<TItem> = {
  readonly items: readonly TItem[];
  readonly cursor?: string;
};
export type MarkedUnreadPagingError =
  | { readonly kind: 'invalid-page-bound'; readonly maxPages: number }
  | { readonly kind: 'page-bound'; readonly maxPages: number }
  | { readonly kind: 'repeated-cursor'; readonly cursor: string }
  | { readonly kind: 'fetch-failed'; readonly cause: unknown };
export type MarkedUnreadResult<TItem> =
  | { readonly kind: 'items'; readonly items: readonly TItem[] }
  | { readonly kind: 'empty' }
  | { readonly kind: 'error'; readonly error: MarkedUnreadPagingError; readonly items: readonly TItem[] };

/** Empty is authoritative only after cursor exhaustion. Never infer it from a
 * filtered page: owned reminders may appear beyond any number of empty pages.
 * Failures preserve partial rows but cannot masquerade as a complete/empty list.
 */
export const readMarkedUnread = async <TItem>(
  fetchPage: (cursor: string | undefined) => Promise<MarkedUnreadPage<TItem>>,
  maxPages = 20,
): Promise<MarkedUnreadResult<TItem>> => {
  const items: TItem[] = [];
  if (!Number.isSafeInteger(maxPages) || maxPages < 1)
    return { kind: 'error', error: { kind: 'invalid-page-bound', maxPages }, items };
  const cursors = new Set<string>();
  let cursor: string | undefined;
  for (let page = 0; page < maxPages; page++) {
    let next: MarkedUnreadPage<TItem>;
    try { next = await fetchPage(cursor); }
    catch (cause) { return { kind: 'error', error: { kind: 'fetch-failed', cause }, items }; }
    items.push(...next.items);
    if (next.cursor === undefined) return items.length ? { kind: 'items', items } : { kind: 'empty' };
    if (cursors.has(next.cursor))
      return { kind: 'error', error: { kind: 'repeated-cursor', cursor: next.cursor }, items };
    cursors.add(next.cursor);
    cursor = next.cursor;
  }
  return { kind: 'error', error: { kind: 'page-bound', maxPages }, items };
};
