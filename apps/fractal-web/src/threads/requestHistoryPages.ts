import type { RequestHistoryItem, RequestHistoryPage, RequestHistoryRelation, SourceEpisode } from '@fractal/client/schema/read'
import { Option, Schema } from 'effect'

export class RequestHistoryPagingError extends Schema.TaggedError<RequestHistoryPagingError>()('RequestHistoryPagingError', {
  reason: Schema.Literals(['repeated-cursor', 'page-bound-exceeded']),
  pages: Schema.Number,
  message: Schema.String,
}) {}

export const sourceEpisodeKey = (key: SourceEpisode): string => JSON.stringify([key.source_id, key.episode])

/** Consume only the authorized generated projection, never infer history from current source data. */
export const readRequestHistoryPages = async (
  readPage: (cursor?: string) => Promise<RequestHistoryPage>,
  maxPages = 100,
): Promise<RequestHistoryPage> => {
  const cursors = new Set<string>()
  const items = new Map<string, RequestHistoryItem>()
  const boundaryItems = new Map<string, RequestHistoryItem>()
  const relations = new Map<string, RequestHistoryRelation>()
  const groups = new Map<string, Map<string, SourceEpisode>>()
  let cursor: string | undefined
  let complete = true
  let sourceSupport: RequestHistoryPage['source_support'] = Option.none()
  for (let pages = 0; pages < maxPages; pages++) {
    const page = await readPage(cursor)
    complete &&= page.complete
    if (Option.isSome(page.source_support)) sourceSupport = page.source_support
    for (const item of page.boundary_items) boundaryItems.set(sourceEpisodeKey(item.key), item)
    for (const item of page.items) items.set(sourceEpisodeKey(item.key), item)
    for (const relation of page.relations) {
      relations.set(JSON.stringify([relation.predecessor, relation.request, relation.reason]), relation)
    }
    for (const group of page.source_episodes) {
      let episodes = groups.get(group.source_id)
      if (episodes === undefined) {
        episodes = new Map()
        groups.set(group.source_id, episodes)
      }
      for (const episode of group.episodes) episodes.set(sourceEpisodeKey(episode), episode)
    }
    const next = Option.getOrUndefined(page.next_cursor)
    // An empty page may still lead to authorized history. Only the terminal cursor ends the read.
    if (next === undefined) {
      for (const key of items.keys()) boundaryItems.delete(key)
      return {
        items: [...items.values()],
        boundary_items: [...boundaryItems.values()],
        relations: [...relations.values()],
        source_episodes: [...groups].map(([source_id, episodes]) => ({ source_id, episodes: [...episodes.values()] })),
        source_support: sourceSupport,
        next_cursor: Option.none(),
        complete,
      }
    }
    if (cursors.has(next)) throw new RequestHistoryPagingError({ reason: 'repeated-cursor', pages: pages + 1, message: 'Fractal repeated a request-history page. Try again.' })
    cursors.add(next)
    cursor = next
  }
  throw new RequestHistoryPagingError({ reason: 'page-bound-exceeded', pages: maxPages, message: 'The request-history page limit was reached. Try again.' })
}
