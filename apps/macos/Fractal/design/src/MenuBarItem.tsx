import { useAppearance } from './appearance.tsx'
import { RenderFigure } from './RenderedPanel.tsx'
import { menuBarStateById, resolveMenuBar, type Appearance } from './renders.ts'

/** Menu-bar variants TrayGallery captures. */
export const menuBarIds = ['M1', 'M2', 'M3'] as const

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
  /** Menu-bar variant: M1 badge, M2 symbol only, M3 count text. */
  readonly menuBar: string
  /** Fixture state the status item shows. */
  readonly state: string
  /** Forced appearance; by default the Storybook appearance. */
  readonly appearance?: Appearance
}

/** The real status item, captured in the menu bar over the gallery backdrop (other items masked). */
export const MenuBarItem = ({ menuBar, state, appearance }: MenuBarItemProps) => {
  const storyAppearance = useAppearance()
  const resolved = appearance ?? storyAppearance
  return (
    <RenderFigure
      image={resolveMenuBar({ menuBar, state, appearance: resolved })}
      title={`${menuBar} · ${menuBarStateById(state)?.title ?? state}`}
      appearance={resolved}
      missing={`menubar-${menuBar}-${state}-${resolved}`}
    />
  )
}
