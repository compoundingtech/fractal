import { quotaGeometry } from './quota-geometry.stylex.ts'

const visibleBox = (element: Element, row: HTMLElement): boolean => {
  for (let current: Element | null = element; current !== null && current !== row; current = current.parentElement) {
    const style = getComputedStyle(current)
    if (style.display === 'none' || style.visibility === 'hidden' || style.clipPath !== 'none' || (style.clip !== 'auto' && style.clip !== '')) return false
  }
  const box = element.getBoundingClientRect()
  return box.width > 0 && box.height > 0
}

/** Measure actual text fragments and SVGs, not only the table's forced row box. */
export const assertQuotaRowLine = (row: HTMLElement, tolerance: number): void => {
  const centres: number[] = []
  const walker = row.ownerDocument.createTreeWalker(row, NodeFilter.SHOW_TEXT)
  while (walker.nextNode()) {
    const text = walker.currentNode
    if (!text.textContent?.trim() || text.parentElement === null || !visibleBox(text.parentElement, row)) continue
    const range = row.ownerDocument.createRange()
    range.selectNodeContents(text)
    for (const box of range.getClientRects()) if (box.width > 0 && box.height > 0) centres.push(box.top + box.height / 2)
  }
  for (const svg of row.querySelectorAll('svg')) if (visibleBox(svg, row)) {
    const box = svg.getBoundingClientRect()
    centres.push(box.top + box.height / 2)
  }
  if (centres.length === 0) throw new Error('Account row has no visible text or glyphs')
  if (Math.max(...centres) - Math.min(...centres) > tolerance) throw new Error('Account content wraps or has a misaligned glyph')
}

/** Account identity and governing evidence keep their own readable columns under pressure. */
export const assertQuotaPriority = (drawer: HTMLElement): void => {
  for (const row of drawer.querySelectorAll<HTMLElement>('[data-quota-row]')) {
    for (const column of ['account', 'primary', 'reset', 'more']) {
      const cell = row.querySelector<HTMLElement>(`[data-quota-cell="${column}"]`)
      if (cell === null || !visibleBox(cell, row)) throw new Error(`Primary quota column ${column} disappeared`)
    }
    const label = row.querySelector<HTMLElement>('[data-quota-account-label]')
    const accountCell = row.querySelector<HTMLElement>('[data-quota-cell="account"]')!
    if (label === null || label.textContent !== row.dataset.quotaAccountLabel || accountCell.clientWidth < label.textContent.length) throw new Error('Account identity lost its readable slot')
    const reset = row.querySelector<HTMLElement>('[data-quota-reset-reading]')
    const resetCell = row.querySelector<HTMLElement>('[data-quota-cell="reset"]')!
    if (reset === null || !visibleBox(reset, row) || reset.getBoundingClientRect().right > resetCell.getBoundingClientRect().right + 1) throw new Error('Governing reset is hidden or clipped')
    const more = row.querySelector<HTMLButtonElement>('[data-quota-more]')
    if (more === null || !more.getAttribute('aria-label')?.includes('amount and reset evidence')) throw new Error('Secondary evidence has no accessible disclosure')
  }
}

/** Browser geometry proof shared by story plays and the negative-control runner. */
export function assertQuotaDrawerDensity(drawer: HTMLElement) {
  const rows = [...drawer.querySelectorAll<HTMLElement>('[data-quota-row]')]
  if (rows.length !== 11) throw new Error(`Density fixture must contain 11 accounts, got ${rows.length}`)
  const headers = [...drawer.querySelectorAll<HTMLElement>('[data-quota-column]')]
  const table = drawer.querySelector<HTMLElement>('table')
  const scroll = drawer.querySelector<HTMLElement>('[data-quota-scroll]')
  if (table === null || scroll === null || headers.length === 0) throw new Error('Missing real quota table/header/scroll geometry')
  if (getComputedStyle(table).display !== 'table') throw new Error('Quota table must use native table layout')
  let maxOffset = 0
  const heights: number[] = []
  const tolerance = Number.parseFloat(getComputedStyle(drawer).getPropertyValue(quotaGeometry.lineCentreTolerance.slice(4, -1)))
  if (!Number.isFinite(tolerance)) throw new Error('Missing quota line-centre tolerance token')
  for (const row of rows) {
    const height = row.getBoundingClientRect().height
    heights.push(height)
    if (height < 20 || height > 22.1) throw new Error(`Account row height ${height}px is outside 20–22px`)
    assertQuotaRowLine(row, tolerance)
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
  assertQuotaPriority(drawer)
  if (scroll.scrollHeight > scroll.clientHeight + 1) throw new Error(`11 accounts must fit without scrolling: ${scroll.scrollHeight}px > ${scroll.clientHeight}px`)
  const last = rows[rows.length - 1]!.getBoundingClientRect()
  if (last.bottom > scroll.getBoundingClientRect().bottom + 1) throw new Error('Final account must fit inside the drawer viewport')
  return { accounts: rows.length, heights, maxOffset, scrollHeight: scroll.scrollHeight, viewportHeight: scroll.clientHeight }
}

/** Native titles and full screen-reader values survive visual truncation. */
export const assertQuotaDrawerDisclosure = (drawer: HTMLElement): void => {
  for (const header of drawer.querySelectorAll<HTMLElement>('[data-quota-column]')) {
    const label = header.querySelector<HTMLElement>('[title]')
    const full = label?.querySelector<HTMLElement>('[data-quota-full-value]')?.textContent ?? label?.textContent
    if (label === null || !label.title || full !== label.title) throw new Error('Missing full header disclosure')
  }
  for (const row of drawer.querySelectorAll<HTMLElement>('[data-quota-row]')) {
    const account = row.querySelector<HTMLButtonElement>('[data-quota-cell="account"] button')
    const caption = account?.querySelector<HTMLElement>('[title]')
    if (account === null || caption === null || caption === undefined || !caption.title.includes(caption.textContent ?? '') || !account.getAttribute('aria-label')?.includes(caption.title)) throw new Error('Missing full account disclosure')
    for (const column of ['amount', 'reset']) {
      const value = row.querySelector<HTMLElement>(`[data-quota-cell="${column}"] > [title]`)
      const full = value?.querySelector<HTMLElement>('[data-quota-full-value]')
      if (value === null || !value.title || full?.textContent !== value.title) throw new Error(`Missing full-value disclosure: ${column}`)
    }
  }
}
