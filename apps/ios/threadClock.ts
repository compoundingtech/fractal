import { ago } from "@fractal/views";

let now = Date.now();
let timer: ReturnType<typeof setInterval> | undefined;
const listeners = new Set<() => void>();

// All mounted thread timelines share one second timer, released with the last reader.
export const threadClock = {
  getSnapshot: (): number => now,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    if (timer === undefined) {
      now = Date.now();
      timer = setInterval(() => {
        now = Date.now();
        for (const notify of listeners) notify();
      }, 1_000);
      listener();
    }
    return () => {
      listeners.delete(listener);
      if (!listeners.size && timer !== undefined) {
        clearInterval(timer);
        timer = undefined;
      }
    };
  },
};

export function threadRelativeTime(acceptedAt: number, currentTime: number): string {
  return `${ago(new Date(acceptedAt).toISOString(), currentTime)} ago`;
}

export function threadExactTime(acceptedAt: number): string {
  return new Date(acceptedAt).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
