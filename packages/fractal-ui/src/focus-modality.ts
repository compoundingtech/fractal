/**
 * Marks the last input modality on the root element, so `tokens.css` can drop focus rings a pointer
 * press should not show. Chromium keeps `:focus-visible` when a press lands on the element already
 * holding keyboard or programmatic focus, because focus does not move; React Aria's
 * `[data-focus-visible]` clears but the native pseudo-class does not.
 *
 * Modifier keys alone and browser chords (Cmd+Tab, Ctrl+C) do not count as keyboard use, matching
 * React Aria. Screen-reader activations synthesise a zero-size pointer and leave the state alone.
 */
const isVirtualPointer = (event: PointerEvent) =>
  (event.width === 0 && event.height === 0) || (event.width === 1 && event.height === 1 && event.pressure === 0 && event.detail === 0)

const isKeyboardUse = (event: KeyboardEvent) =>
  !event.metaKey && !event.ctrlKey && !event.altKey && event.key !== 'Shift' && event.key !== 'Control' && event.key !== 'Meta' && event.key !== 'Alt'

export function installFocusModality(root: HTMLElement = document.documentElement) {
  const doc = root.ownerDocument
  const onKey = (event: KeyboardEvent) => { if (isKeyboardUse(event)) root.dataset.inputModality = 'keyboard' }
  const onPointer = (event: PointerEvent) => { if (!isVirtualPointer(event)) root.dataset.inputModality = 'pointer' }
  doc.addEventListener('keydown', onKey, true)
  doc.addEventListener('pointerdown', onPointer, true)
  return () => {
    doc.removeEventListener('keydown', onKey, true)
    doc.removeEventListener('pointerdown', onPointer, true)
    delete root.dataset.inputModality
  }
}
