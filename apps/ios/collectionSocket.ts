import type { CollectionSocket } from '../../clients/typescript/st3-client';

export type HeartbeatSocket = CollectionSocket & { onping: (() => void) | null };
export type HeartbeatSocketFactory = (url: string, protocols: string[], headers: Record<string, string>) => HeartbeatSocket;
export type SocketEvent = { id: number; kind: 'open' | 'message' | 'ping' | 'error' | 'close'; data?: string; code?: number; reason?: string };
export type SocketBridge = {
  open(id: number, url: string, protocols: string[], headers: Record<string, string>): Promise<void> | void;
  send(id: number, data: string): Promise<void> | void;
  close(id: number, code: number): Promise<void> | void;
  addListener(name: 'socket', listener: (event: SocketEvent) => void): { remove(): void };
};

/** SocketRocket performs the ordinary WebSocket handshake and automatic pong. Only its events
 * cross this bridge, including protocol ping, which React Native's WebSocket discards. */
export function collectionSocketFactory(bridge: SocketBridge): HeartbeatSocketFactory {
  let nextId = 0;
  return (url, protocols, headers) => {
    const id = ++nextId;
    let closed = false;
    const failed = () => { if (closed) return; socket.onerror?.({}); socket.close(); };
    const socket: HeartbeatSocket = {
      onopen: null, onmessage: null, onclose: null, onerror: null, onping: null,
      send: data => { if (!closed) void Promise.resolve(bridge.send(id, data)).catch(failed); },
      close: (code = 1000) => { if (closed) return; closed = true; listener.remove(); void Promise.resolve(bridge.close(id, code)).catch(() => {}); },
    };
    const listener = bridge.addListener('socket', event => {
      if (closed || event.id !== id) return;
      if (event.kind === 'open') socket.onopen?.();
      else if (event.kind === 'message') socket.onmessage?.({ data: event.data });
      else if (event.kind === 'ping') socket.onping?.();
      else {
        closed = true; listener.remove();
        if (event.kind === 'error') socket.onerror?.({});
        else socket.onclose?.({ code: event.code ?? 1006, reason: event.reason ?? '' });
      }
    });
    // The adapter is returned before any native open event is delivered.
    queueMicrotask(() => { if (!closed) void Promise.resolve(bridge.open(id, url, protocols, headers)).catch(failed); });
    return socket;
  };
}
