/** Browser focus and visibility are independent: a visible background tab must not execute. */
export const isClientControlForeground = (document: Pick<Document, 'visibilityState' | 'hasFocus'>): boolean =>
  document.visibilityState === 'visible' && document.hasFocus()

/** Re-read immediately before navigation and again before signing; never cache this verdict. */
export const browserClientControlForeground = (): boolean => isClientControlForeground(document)
