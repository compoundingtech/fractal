import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { HumanClockProvider, fixedClock } from '../time/HumanTime'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from '../assistant-ui/composition-theme'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { notesNow, notesZone } from './notes-fixtures'
import type { ArgTypes } from '@storybook/react-vite'
import { expect } from 'storybook/test'

/** The agent marker is a painted SVG: present, laid out, visible and drawn in a non-transparent stroke. */
export const expectVisibleAgentMarker = async (scope: Element) => {
  const marker = scope.querySelector<SVGSVGElement>('svg[data-note-agent-marker]')
  await expect(marker).not.toBeNull()
  const box = marker!.getBoundingClientRect()
  await expect(box.width).toBeGreaterThan(0)
  await expect(box.height).toBeGreaterThan(0)
  const style = getComputedStyle(marker!)
  await expect(style.visibility).toBe('visible')
  await expect(style.display).not.toBe('none')
  await expect(style.stroke).not.toMatch(/^(none|transparent|rgba\(0, 0, 0, 0\))$/)
}

/** Shared args for notes stories: scheme, polish width and the per-surface negative control. */
export interface NotesFrameArgs {
  readonly scheme: Scheme
  readonly width: 440 | 1440
  /** Off in every exported story; turning it on must make that story's play fail. */
  readonly negativeControl: boolean
}
export const notesFrameArgs: NotesFrameArgs = { scheme: 'dark', width: 440, negativeControl: false }
export const notesFrameArgTypes = (negativeControl: string): Partial<ArgTypes<NotesFrameArgs>> => ({
  scheme: { options: ['dark', 'light'], control: 'radio' },
  width: { options: [440, 1440], control: 'select' },
  negativeControl: { control: 'boolean', description: negativeControl },
})

/** Themed 440/1440 canvas with a fixed clock pinned to UTC, so time labels are deterministic. */
export function NotesFrame({ scheme, width, title, children }: Pick<NotesFrameArgs, 'scheme' | 'width'> & { readonly title: string; readonly children: React.ReactNode }) {
  return <main data-testid="notes-story" data-scheme={scheme} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.root, width === 1440 ? styles.wide : styles.narrow)}>
    <HumanClockProvider clock={fixedClock(notesNow)} zone={notesZone}>
      <h1 {...stylex.props(styles.title)}>{title}</h1>
      {children}
    </HumanClockProvider>
  </main>
}

export const notesStoryStyles = stylex.create({
  section: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0 },
  heading: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightSemibold, color: text.fgMuted },
  row: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, minWidth: 0 },
})

const styles = stylex.create({
  root: {
    minHeight: '100vh', boxSizing: 'border-box', maxWidth: '100%', padding: s.xl, display: 'flex', flexDirection: 'column', gap: s.xl,
    backgroundColor: surface.canvas, color: text.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading,
  },
  narrow: { width: g.polishNarrow },
  wide: { width: g.polishWide },
  title: { margin: s.zero, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightSemibold },
})
