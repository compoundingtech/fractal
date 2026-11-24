import { useAppearance } from './appearance.tsx'
import { RenderFigure } from './RenderedPanel.tsx'
import { menuBarStateById, resolveMenuBar, type Appearance } from './renders.ts'

/** Status-item fixture states, in gallery order. */
export const menuBarStateIds = [
  'idle',
  'one',
  'several',
  'many',
  'urgent',
  'offline',
  'unconfigured',
] as const

/** Props of {@link MenuBarItem}. */
export interface MenuBarItemProps {
  /** Fixture state the status item shows. */
  readonly state: string
  /** Forced appearance; by default the Storybook appearance. */
  readonly appearance?: Appearance
}

/** The real status item, captured in the menu bar over the gallery backdrop (other items masked). */
export const MenuBarItem = ({ state, appearance }: MenuBarItemProps) => {
  const storyAppearance = useAppearance()
  const resolved = appearance ?? storyAppearance
  return (
    <RenderFigure
      image={resolveMenuBar({ state, appearance: resolved })}
      title={menuBarStateById(state)?.title ?? state}
      appearance={resolved}
      missing={`menubar-${state}-${resolved}`}
    />
  )
}
