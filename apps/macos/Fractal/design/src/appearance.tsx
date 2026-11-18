import { createContext, useContext, useEffect, useSyncExternalStore, type ReactNode } from 'react'

import type { Appearance } from './renders.ts'

/** The toolbar choice: follow macOS, or force one appearance. */
export type AppearanceChoice = 'system' | Appearance

const darkQuery = '(prefers-color-scheme: dark)'

const subscribe = (onChange: () => void) => {
  const media = window.matchMedia(darkQuery)
  media.addEventListener('change', onChange)
  return () => media.removeEventListener('change', onChange)
}

/** The viewer's OS appearance, updating live when macOS switches. */
export const useSystemAppearance = (): Appearance =>
  useSyncExternalStore(
    subscribe,
    () => (window.matchMedia(darkQuery).matches ? 'dark' : 'light'),
    () => 'light',
  )

const AppearanceContext = createContext<Appearance>('light')

/** The appearance stories render in: the toolbar override, else the OS appearance. */
export const useAppearance = (): Appearance => useContext(AppearanceContext)

/**
 * Resolves the toolbar choice and applies it to the canvas (`data-appearance`, `color-scheme`),
 * so the surround matches the renders it frames.
 */
export const AppearanceProvider = ({
  choice,
  children,
}: {
  choice: AppearanceChoice | undefined
  children: ReactNode
}) => {
  const system = useSystemAppearance()
  const appearance = choice === 'light' || choice === 'dark' ? choice : system
  useEffect(() => {
    document.documentElement.dataset['appearance'] = appearance
    document.documentElement.style.colorScheme = appearance
  }, [appearance])
  return <AppearanceContext.Provider value={appearance}>{children}</AppearanceContext.Provider>
}
