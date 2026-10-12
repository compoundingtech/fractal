/** Self-contained for story plays and the standalone runner's page.evaluate call. */
export function assertQuotaDrawerDensity(drawer: HTMLElement) {
  const visibleBox = (element: Element, row: HTMLElement): boolean => {
    for (let current: Element | null = element; current !== null && current !== row; current = current.parentElement) {
      const style = getComputedStyle(current)
      if (style.display === 'none' || style.visibility === 'hidden' || style.clipPath !== 'none' || (style.clip !== 'auto' && style.clip !== '')) return false
    }
    const box = element.getBoundingClientRect()
    return box.width > 0 && box.height > 0
  }
  const rows = drawer.querySelectorAll<HTMLElement>('[data-quota-row]')
  if (rows.length !== 11) throw new Error(`Density fixture must contain 11 accounts, got ${rows.length}`)
  const headers = drawer.querySelectorAll<HTMLElement>('[data-quota-column]')
  const table = drawer.querySelector<HTMLElement>('table')
  const scroll = drawer.querySelector<HTMLElement>('[data-quota-scroll]')
  if (table === null || scroll === null || headers.length === 0) throw new Error('Missing real quota table/header/scroll geometry')
  if (getComputedStyle(table).display !== 'table') throw new Error('Quota table must use native table layout')
  const variable = drawer.dataset.quotaLineTolerance
  if (variable === undefined) throw new Error('Missing quota line-centre tolerance token reference')
  const tolerance = Number.parseFloat(getComputedStyle(drawer).getPropertyValue(variable.slice(4, -1)))
  if (!Number.isFinite(tolerance)) throw new Error('Missing quota line-centre tolerance token')
  const range = drawer.ownerDocument.createRange()
  let maxOffset = 0
  const heights: number[] = []
  for (const row of rows) {
    const height = row.getBoundingClientRect().height
    heights.push(height)
    if (height < 20 || height > 22.1) throw new Error(`Account row height ${height}px is outside 20–22px`)
    // Measure actual text fragments and SVGs, not only the forced row box.
    let firstCentre = Infinity
    let lastCentre = -Infinity
    const walker = drawer.ownerDocument.createTreeWalker(row, NodeFilter.SHOW_TEXT)
    while (walker.nextNode()) {
      const text = walker.currentNode
      if (!text.textContent?.trim() || text.parentElement === null || !visibleBox(text.parentElement, row)) continue
      range.selectNodeContents(text)
      for (const box of range.getClientRects()) if (box.width > 0 && box.height > 0) {
        const centre = box.top + box.height / 2
        firstCentre = Math.min(firstCentre, centre)
        lastCentre = Math.max(lastCentre, centre)
      }
    }
    for (const svg of row.querySelectorAll('svg')) if (visibleBox(svg, row)) {
      const box = svg.getBoundingClientRect()
      const centre = box.top + box.height / 2
      firstCentre = Math.min(firstCentre, centre)
      lastCentre = Math.max(lastCentre, centre)
    }
    if (!Number.isFinite(firstCentre)) throw new Error('Account row has no visible text or glyphs')
    if (lastCentre - firstCentre > tolerance) throw new Error('Account content wraps or has a misaligned glyph')
    for (const column of ['account', 'primary', 'reset', 'more']) {
      const cell = row.querySelector<HTMLElement>(`[data-quota-cell="${column}"]`)
      if (cell === null || !visibleBox(cell, row)) throw new Error(`Primary quota column ${column} disappeared`)
    }
    const label = row.querySelector<HTMLElement>('[data-quota-account-label]')
    const accountCell = row.querySelector<HTMLElement>('[data-quota-cell="account"]')!
    if (label === null || label.textContent !== row.dataset.quotaAccountLabel || accountCell.clientWidth < label.textContent.length) throw new Error('Account identity lost its readable slot')
    // Priority: secondary columns (amount, extra windows) only show once the account fits; its provider suffix folds whole, never mid-word.
    const accountButton = accountCell.querySelector<HTMLElement>('button')
    const secondaryShown = [...row.querySelectorAll<HTMLElement>('[data-quota-cell]')].some(cell => !['account', 'primary', 'reset', 'more'].includes(cell.dataset.quotaCell ?? '') && visibleBox(cell, row))
    if (secondaryShown && (accountButton === null || accountButton.scrollWidth > accountButton.clientWidth + 0.5)) throw new Error(`Account ${row.dataset.quotaAccountLabel} is cut while secondary columns show`)
    const provider = row.querySelector<HTMLElement>('[data-quota-account-provider]')
    if (provider !== null && accountButton !== null && visibleBox(provider, row) && provider.getBoundingClientRect().right > accountButton.getBoundingClientRect().right + 0.5) throw new Error('Account provider suffix is cut mid-word')
    const reset = row.querySelector<HTMLElement>('[data-quota-reset-reading]')
    const resetCell = row.querySelector<HTMLElement>('[data-quota-cell="reset"]')!
    if (reset === null || !visibleBox(reset, row) || reset.getBoundingClientRect().right > resetCell.getBoundingClientRect().right + 1) throw new Error('Governing reset is hidden or clipped')
    const more = row.querySelector<HTMLButtonElement>('[data-quota-more]')
    if (more === null || !more.querySelector('[data-quota-more-description]')?.textContent?.includes('amount and reset evidence')) throw new Error('Secondary evidence has no accessible disclosure')
    const cells = [...row.querySelectorAll<HTMLElement>('[data-quota-cell]')]
    if (cells.length !== headers.length) throw new Error('Account cells must match table header columns')
    for (const header of headers) {
      const cell = cells.find(cell => cell.dataset.quotaCell === header.dataset.quotaColumn)
      if (cell === undefined) throw new Error(`Missing cell for ${header.dataset.quotaColumn}`)
      const offset = Math.abs(cell.getBoundingClientRect().x - header.getBoundingClientRect().x)
      maxOffset = Math.max(maxOffset, offset)
      if (offset > 1) throw new Error(`Column ${header.dataset.quotaColumn} misalignment ${offset}px exceeds 1px`)
      if (getComputedStyle(header).position !== 'sticky') throw new Error('Quota header must remain sticky')
    }
  }
  if (scroll.scrollHeight > scroll.clientHeight + 1) throw new Error(`11 accounts must fit without scrolling: ${scroll.scrollHeight}px > ${scroll.clientHeight}px`)
  const last = rows[rows.length - 1]!.getBoundingClientRect()
  if (last.bottom > scroll.getBoundingClientRect().bottom + 1) throw new Error('Final account must fit inside the drawer viewport')
  return { accounts: rows.length, heights, maxOffset, scrollHeight: scroll.scrollHeight, viewportHeight: scroll.clientHeight }
}

/** Full values stay mounted for screen readers and name their focusable tooltip triggers; no native title doubles a custom hover (React Aria's hidden `<select title="">` shows none). */
export const assertQuotaDrawerDisclosure = (drawer: HTMLElement): void => {
  if (drawer.querySelector('[title]:not([title=""])') !== null) throw new Error('Native title duplicates a custom disclosure')
  for (const header of drawer.querySelectorAll<HTMLElement>('[data-quota-column]')) {
    const full = header.querySelector<HTMLElement>('[data-quota-full-value]')?.textContent ?? header.textContent
    if (!full?.trim()) throw new Error('Missing full header disclosure')
  }
  for (const row of drawer.querySelectorAll<HTMLElement>('[data-quota-row]')) {
    const account = row.querySelector<HTMLButtonElement>('[data-quota-cell="account"] button')
    const label = account?.querySelector<HTMLElement>('[data-quota-account-label]')?.textContent
    if (account === null || !label || !account.getAttribute('aria-label')?.includes(label)) throw new Error('Missing full account disclosure')
    for (const column of ['amount', 'reset']) {
      const trigger = row.querySelector<HTMLElement>(`[data-quota-cell="${column}"] > [data-full-value-trigger]`)
      const value = trigger?.querySelector<HTMLElement>('[data-quota-full-value]')
      if (trigger === null || value === null || value === undefined || trigger.tabIndex !== 0 || !value.textContent?.trim() || trigger.getAttribute('aria-labelledby') !== value.id || trigger.firstElementChild?.getAttribute('aria-hidden') !== 'true') throw new Error(`Missing full-value disclosure: ${column}`)
    }
  }
}
