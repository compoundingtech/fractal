import { expect, userEvent, waitFor } from 'storybook/test'

/** A native title would add a second, unstyled hover on top of the custom disclosure. React Aria's hidden form `<select title="">` shows none. */
export const expectNoNativeTitles = async (scope: Element): Promise<void> => {
  await expect([...scope.querySelectorAll('[title]:not([title=""])')].map(node => node.outerHTML.slice(0, 160))).toEqual([])
}

/** Hover, hover plus keyboard focus, and keyboard focus alone each show exactly one tooltip with the full value, asserted after React re-renders. */
export const expectSingleFullValueTooltip = async (trigger: HTMLElement, full: string): Promise<void> => {
  const tooltips = () => [...trigger.ownerDocument.querySelectorAll<HTMLElement>('[role="tooltip"]')]
  // The 350ms open delay plus a re-render can exceed waitFor's 1s default on a loaded CI runner.
  const settle = { timeout: 5000 }
  const expectOne = () => waitFor(() => {
    expect(tooltips()).toHaveLength(1)
    expect(tooltips()[0]).toHaveTextContent(full)
    // React Aria links the open tooltip as the trigger's accessible description.
    expect(trigger.getAttribute('aria-describedby')?.split(' ')).toContain(tooltips()[0]!.id)
    // A readable horizontal box inside the viewport, not a shrink-to-fit column of single glyphs.
    const box = tooltips()[0]!.getBoundingClientRect(), viewport = trigger.ownerDocument.documentElement
    expect(box.width).toBeGreaterThan(box.height)
    expect(box.left >= 0 && box.top >= 0 && box.right <= viewport.clientWidth && box.bottom <= viewport.clientHeight).toBe(true)
  }, settle)
  // A pointer can only rest on a visible value; React Aria opens on hover only in pointer modality, and a press on the empty body sets it without activating anything.
  trigger.scrollIntoView({ block: 'center', inline: 'nearest' })
  await userEvent.pointer({ keys: '[MouseLeft]', target: trigger.ownerDocument.body })
  await userEvent.hover(trigger)
  await expectOne()
  // Any non-modifier key puts React Aria in keyboard modality, so programmatic focus counts as visible focus.
  await userEvent.keyboard('{F13}')
  trigger.focus()
  await expectOne()
  // React Aria closes on pointer leave even while focused; refocusing reopens it.
  await userEvent.unhover(trigger)
  await waitFor(() => expect(tooltips()).toHaveLength(0), settle)
  trigger.blur()
  await userEvent.keyboard('{F13}')
  trigger.focus()
  await expectOne()
  trigger.blur()
  await waitFor(() => expect(tooltips()).toHaveLength(0), settle)
}
