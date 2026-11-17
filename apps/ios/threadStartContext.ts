import type { FractalClient, SourceEpisode, ThreadStartContext } from '../../clients/typescript/fractal-client';
import { ClientError } from '../../clients/typescript/fractal-client';

export type ThreadStartContextState =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'blocked'; reason: 'recipient-unassigned' | 'recipient-retired'; message: string }
  | { kind: 'ready'; context: ThreadStartContext; changed: boolean };

/** Server-selected first-send audience. A refresh never sends or edits a signed request.
 * The UI must show the newly read recipient before a separate, explicit user send. */
export class ThreadStartContextReader {
  private state: ThreadStartContextState = { kind: 'loading' };
  private listeners = new Set<() => void>();
  private active?: AbortController;
  private generation = 0;
  private lastRecipient?: string;

  constructor(private readonly client: Pick<FractalClient, 'threadStartContext'>, private readonly origin: SourceEpisode) {}

  getSnapshot = (): ThreadStartContextState => this.state;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  private publish = (state: ThreadStartContextState): void => {
    this.state = state;
    for (const listener of this.listeners) listener();
  };
  load = async (refresh = false): Promise<void> => {
    this.active?.abort();
    const request = new AbortController();
    this.active = request;
    const generation = ++this.generation;
    this.publish({ kind: 'loading' });
    try {
      const { value } = await this.client.threadStartContext(this.origin.source_id, this.origin.episode, { signal: request.signal });
      if (request.signal.aborted || generation !== this.generation) return;
      if (value.source_id !== this.origin.source_id || value.episode !== this.origin.episode) {
        this.publish({ kind: 'error', message: 'The Start recipient is unavailable for this request. Refresh to try again.' });
        return;
      }
      const changed = refresh || this.lastRecipient !== undefined && this.lastRecipient !== value.start_recipient.agent_id;
      this.lastRecipient = value.start_recipient.agent_id;
      this.publish({ kind: 'ready', context: value, changed });
    } catch (error) {
      if (request.signal.aborted || generation !== this.generation) return;
      if (error instanceof ClientError && (error.response.code === 'recipient-unassigned' || error.response.code === 'recipient-retired')) {
        this.publish({ kind: 'blocked', reason: error.response.code, message: error.response.code === 'recipient-unassigned'
          ? 'No Start recipient is configured. Configure a responsible agent before starting a discussion. Your draft is unchanged.'
          : 'This Start recipient is retired and no replacement is available. Your draft is unchanged.' });
        return;
      }
      this.publish({ kind: 'error', message: 'The Start recipient is unavailable. Refresh to try again. Your draft is unchanged.' });
    }
  };
  close = (): void => {
    this.generation++;
    this.active?.abort();
    this.active = undefined;
    this.lastRecipient = undefined;
    this.publish({ kind: 'loading' });
  };
}
