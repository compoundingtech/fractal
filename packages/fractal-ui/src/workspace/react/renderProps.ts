import type * as React from 'react'

/**
 * Styling the React Aria Components way: `className` and `style` as values or as functions of the
 * component's render state, `children` as nodes or a function of it, and state as `data-*` attributes.
 */
export interface StyleRenderProps<T> {
  readonly className?: string | ((values: T) => string | undefined) | undefined
  readonly style?: React.CSSProperties | ((values: T) => React.CSSProperties | undefined) | undefined
}

export interface RenderProps<T> extends StyleRenderProps<T> {
  readonly children?: React.ReactNode | ((values: T) => React.ReactNode) | undefined
}

export const resolveClassName = <T>(className: StyleRenderProps<T>['className'], values: T): string | undefined =>
  typeof className === 'function' ? className(values) : className

/** Structural styles first; the host's style wins over them. */
export const resolveStyle = <T>(style: StyleRenderProps<T>['style'], values: T, structural?: React.CSSProperties): React.CSSProperties | undefined => {
  const own = typeof style === 'function' ? style(values) : style
  return structural === undefined ? own : { ...structural, ...own }
}

export const resolveChildren = <T>(children: RenderProps<T>['children'], values: T): React.ReactNode =>
  typeof children === 'function' ? children(values) : children

/** `data-*` attributes for boolean and string state: `true` as present, `false` and `undefined` as absent. */
export const dataAttributes = (state: Readonly<Record<string, boolean | string | number | undefined | null>>): Record<string, string | undefined> => {
  const attributes: Record<string, string | undefined> = {}
  for (const [key, value] of Object.entries(state)) {
    const name = `data-${key.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`)}`
    if (value === true) attributes[name] = ''
    else if (typeof value === 'string' || typeof value === 'number') attributes[name] = String(value)
  }
  return attributes
}
