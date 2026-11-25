import { ClientError, type ActionOf, type ActionResult, type Capabilities, type EnvelopeOf, type FractalClient, type ThreadClearUnreadParameters, type ThreadMarkUnreadParameters, type ThreadSummary, type ThreadTimelineAnchor, type ThreadTimelinePage } from '../../clients/typescript/fractal-client';
import { decodeUnknownSync, ThreadSummaryPage as SummaryPageSchema, ThreadTimelinePage as TimelinePageSchema } from '../../clients/typescript/fractal-client/Schema.generated.ts';
import { pendingUnreadAPI, type UnreadActionResult, type UnreadAPI } from './unreadActions';
import { readMarkedUnread, type MarkedUnreadPagingError } from './unreadPaging';

/** Unread reminder reads and writes were added by threads-v1 version 3. */
export const unreadSupported = (caps: Pick<Capabilities, 'capabilities'> | null | undefined): boolean =>
  !!caps?.capabilities.some(capability => capability.id === 'threads-v1' && capability.version >= 3 && capability.state === 'granted');
export type UnreadRequest = Omit<ActionOf<'thread.mark-unread'>, 'api_version' | 'type' | 'parameters'>;
export type UnreadMutation = 'mark-unread' | 'clear-unread';
export type UnreadReadError =
  | { readonly kind: 'refused'; readonly code: string }
  | { readonly kind: 'invalid-response' }
  | { readonly kind: 'failed'; readonly cause: unknown };
export type UnreadSeek =
  | { readonly state: 'current'; readonly anchor: ThreadTimelineAnchor; readonly page: ThreadTimelinePage }
  | { readonly state: 'unavailable' | 'pending'; readonly anchor: ThreadTimelineAnchor }
  | { readonly state: 'error'; readonly error: UnreadReadError };
export type UnreadList =
  | { readonly kind: 'items'; readonly items: readonly ThreadSummary[] }
  | { readonly kind: 'empty' }
  | { readonly kind: 'error'; readonly items: readonly ThreadSummary[]; readonly error:
      Exclude<MarkedUnreadPagingError, { kind: 'fetch-failed' }> | { readonly kind: 'read-failed'; readonly reason: UnreadReadError } };
export interface UnreadClientPorts {
  /** Live checks, not the grants captured when a menu opened. The server's
   * thread.mark-unread/clear-unread capabilities require control.messages.
   */
  supported(): boolean;
  canRead(): boolean;
  canWrite(kind: UnreadMutation): boolean;
  read(thread: ThreadSummary, ids: readonly string[]): Promise<UnreadActionResult>;
  /** Existing origin-bound signed delivery and original-request retry machinery. */
  submit(target: string, kind: UnreadMutation, send: (client: FractalClient, request: UnreadRequest) => Promise<EnvelopeOf<ActionResult>>): Promise<UnreadActionResult>;
}
class UnreadRefused extends Error {}
class UnreadUpgradeRequired extends Error {}
const readError = (cause: unknown): UnreadReadError => cause instanceof UnreadUpgradeRequired
  ? { kind: 'refused', code: 'upgrade-required' } : cause instanceof UnreadRefused
  ? { kind: 'refused', code: 'forbidden' } : cause instanceof ClientError
  ? { kind: 'refused', code: cause.response.code } : { kind: 'failed', cause };
class InvalidUnreadResponse extends Error {}

/** Generated methods own all wire shapes. Seeking and listing are reads only:
 * they never acknowledge messages or turn an action receipt into a mark ID.
 */
export const unreadClient = (client: FractalClient | null, ports: UnreadClientPorts): {
  forThread(thread: ThreadSummary): UnreadAPI;
  listMarked(maxPages?: number): Promise<UnreadList>;
} => ({
  forThread: thread => !ports.supported() ? pendingUnreadAPI : ({ state: 'ready', actions: {
    read: ids => client && ports.canRead() ? ports.read(thread, ids) : Promise.resolve({ how: 'failed', error: 'This connection cannot acknowledge thread messages.' }),
    edit: edit => {
      const kind = edit.kind === 'mark' ? 'mark-unread' : 'clear-unread';
      if (!ports.supported()) return Promise.resolve({ how: 'failed', error: 'Upgrade the connected Fractal to use unread reminders.' });
      if (!client || !ports.canWrite(kind)) return Promise.resolve({ how: 'failed', error: 'This connection cannot change unread reminders.' });
      // Copy the captured IDs once. A later summary cannot change a bound retry.
      const observed_marks = [...edit.observed_marks];
      if (edit.kind === 'mark') {
        const parameters: ThreadMarkUnreadParameters = { target_id: thread.id, anchor_message_id: edit.anchor_message, observed_marks };
        return ports.submit(thread.id, kind, (member, request) => member.threadMarkUnread({ ...request, parameters }));
      }
      const parameters: ThreadClearUnreadParameters = { target_id: thread.id, observed_marks };
      return ports.submit(thread.id, kind, (member, request) => member.threadClearUnread({ ...request, parameters }));
    },
    seek: async message_id => {
      if (!ports.supported()) return { state: 'error', error: { kind: 'refused', code: 'upgrade-required' } };
      if (!client || !ports.canRead()) return { state: 'error', error: { kind: 'refused', code: 'forbidden' } };
      let page: ThreadTimelinePage;
      try { page = (await client.threadTimeline(thread.id, { around: message_id, limit: 25 })).value; }
      catch (cause) {
        // Addendum 4: generic refusal conveys no existence/intent knowledge.
        // Only an explicit server pending result can produce Pending.
        if (cause instanceof ClientError && cause.response.code === 'thread-unavailable')
          return { state: 'unavailable', anchor: { message_id, state: 'unavailable' } };
        return { state: 'error', error: readError(cause) };
      }
      try { decodeUnknownSync(TimelinePageSchema, 'strict')(page); }
      catch { return { state: 'error', error: { kind: 'invalid-response' } }; }
      const anchor = page.anchor;
      if (!anchor || anchor.message_id !== message_id) return { state: 'error', error: { kind: 'invalid-response' } };
      // Non-current responses include the newest page on the wire. Do NOT hand
      // that unrelated body to an anchor view, or infer a reminder from it.
      return anchor.state === 'current' ? { state: 'current', anchor, page } : { state: anchor.state, anchor };
    },
  } }),
  listMarked: async maxPages => {
    const result = await readMarkedUnread(async cursor => {
      if (!ports.supported()) throw new UnreadUpgradeRequired();
      if (!client || !ports.canRead()) throw new UnreadRefused();
      const { value } = await client.threadList({ marked_unread: true, cursor, limit: 25 });
      // Validate with the canonical decoder, retaining the generated wire DTO
      // rather than its Effect Option representation for nullable fields.
      try { decodeUnknownSync(SummaryPageSchema, 'strict')(value); }
      catch { throw new InvalidUnreadResponse(); }
      return { items: value.items, cursor: value.next_cursor ?? undefined };
    }, maxPages);
    if (result.kind === 'error') {
      const error = result.error;
      return { kind: 'error', items: result.items, error: error.kind === 'fetch-failed'
        ? { kind: 'read-failed', reason: error.cause instanceof InvalidUnreadResponse
          ? { kind: 'invalid-response' } : readError(error.cause) }
        : error };
    }
    return result;
  },
});
