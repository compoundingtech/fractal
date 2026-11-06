/**
 * Context guard for document-level single-key shortcuts (`/` search, `n` note). A plain letter
 * must never steal keys from a widget that types, from a collection whose typeahead jumps to the
 * matching item, from an open menu or dialog that owns the keyboard, or from an IME composition.
 * Callers still check their own key and modifiers.
 */
const editableSelector = 'input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="searchbox"],[role="combobox"]'
/** Collections with letter typeahead (RAC Tree, GridList, ListBox, Menu): a letter moves to the matching item. */
const typeaheadSelector = '[role="tree"],[role="treegrid"],[role="grid"],[role="listbox"],[role="menu"]'
/** Overlays that own the keyboard while open; closed or hidden instances never block. */
const overlaySelector = '[role="dialog"],[role="alertdialog"],[role="menu"],[aria-modal="true"],dialog[open]'

/** True when the event must be left to the focused widget, a typeahead collection, an open overlay or an IME. */
export const shortcutContextBlocked = (event: KeyboardEvent, root: ParentNode = document): boolean => {
  if (event.isComposing || event.keyCode === 229) return true
  const target = event.target
  if (target instanceof HTMLElement && (target.isContentEditable || target.closest(editableSelector) !== null || target.closest(typeaheadSelector) !== null)) return true
  // Open means rendered and exposed: not hidden, inert or aria-hidden, and laid out with a box.
  return Array.from(root.querySelectorAll(overlaySelector)).some(element =>
    element.closest('[hidden],[inert],[aria-hidden="true"],dialog:not([open])') === null && element.getClientRects().length > 0)
}
