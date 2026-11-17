import { createContext, useContext, useMemo, type ReactNode } from 'react';
import { Keyboard, type ScrollView, type View } from 'react-native';
import { intersectThreadBounds, type ThreadBounds } from './threadVisibility';

type Measurable = Pick<View, 'measureInWindow'>;
type ViewportRef = { readonly current: ScrollView | null };

// Expanded bodies measure on real viewport changes, never from a summary/list row.
const listeners = new WeakMap<ViewportRef, Set<() => void>>();
export function threadViewportChanged(viewport: ViewportRef): void {
  listeners.get(viewport)?.forEach(listener => listener());
}
export function watchThreadViewports(viewports: readonly ViewportRef[], changed: () => void): () => void {
  for (const viewport of viewports) {
    let watching = listeners.get(viewport);
    if (!watching) { watching = new Set(); listeners.set(viewport, watching); }
    watching.add(changed);
  }
  return () => { for (const viewport of viewports) listeners.get(viewport)?.delete(changed); };
}

const Viewports = createContext<readonly ViewportRef[]>([]);

export function ThreadViewport({ viewport, children }: { viewport: ViewportRef; children: ReactNode }) {
  const parent = useContext(Viewports);
  const stack = useMemo(() => [...parent, viewport], [parent, viewport]);
  return <Viewports.Provider value={stack}>{children}</Viewports.Provider>;
}

export function useThreadViewports() {
  return useContext(Viewports);
}

export function measureThreadBounds(view: Measurable | null | undefined): Promise<ThreadBounds | undefined> {
  if (!view) return Promise.resolve(undefined);
  return new Promise(resolve => {
    // A view can unmount before React Native delivers the measurement.
    const timer = setTimeout(() => resolve(undefined), 250);
    view.measureInWindow((x, y, width, height) => {
      clearTimeout(timer);
      resolve(width > 0 && height > 0 ? { x, y, width, height } : undefined);
    });
  });
}

export async function measureThreadViewport(
  viewports: readonly ViewportRef[], window: ThreadBounds,
): Promise<ThreadBounds | undefined> {
  if (!viewports.length) return undefined;
  const keyboard = Keyboard.metrics();
  const height = keyboard ? Math.min(window.height, keyboard.screenY - window.y) : window.height;
  const bounds = await Promise.all(viewports.map(viewport => measureThreadBounds(viewport.current?.getNativeScrollRef())));
  return bounds.reduce<ThreadBounds | undefined>(
    (visible, clip) => visible && clip ? intersectThreadBounds(visible, clip) : undefined,
    { ...window, height },
  );
}
