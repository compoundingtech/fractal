/**
 * jsdom has no top layer. Model only the native popover open state the developer-bar kit reads
 * (`showPopover`, `hidePopover`, `:popover-open`); real light dismissal is covered by story plays.
 * Returns the restore function.
 */
export const installNativePopover = (): (() => void) => {
  const open = new WeakSet<Element>()
  const matches = Object.getOwnPropertyDescriptor(Element.prototype, 'matches')!
  const native = {
    showPopover: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'showPopover'),
    hidePopover: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'hidePopover'),
  }
  const nativeMatches = matches.value as (this: Element, selector: string) => boolean
  Object.defineProperty(Element.prototype, 'matches', {
    configurable: true,
    writable: true,
    value(this: Element, selector: string) { return selector === ':popover-open' ? open.has(this) : nativeMatches.call(this, selector) },
  })
  Object.defineProperties(HTMLElement.prototype, {
    showPopover: { configurable: true, writable: true, value(this: HTMLElement) { open.add(this) } },
    hidePopover: { configurable: true, writable: true, value(this: HTMLElement) { open.delete(this) } },
  })
  return () => {
    Object.defineProperty(Element.prototype, 'matches', matches)
    for (const name of ['showPopover', 'hidePopover'] as const) {
      const descriptor = native[name]
      if (descriptor === undefined) Reflect.deleteProperty(HTMLElement.prototype, name)
      else Object.defineProperty(HTMLElement.prototype, name, descriptor)
    }
  }
}
