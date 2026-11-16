import * as React from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

const remarkPlugins = [remarkGfm]
const blocks: Readonly<Record<string, true>> = { p: true, div: true, pre: true, blockquote: true, ul: true, ol: true, li: true, table: true, thead: true, tbody: true, tr: true, th: true, td: true, h1: true, h2: true, h3: true, h4: true, h5: true, h6: true, br: true, hr: true }

/** The same synchronous parser and GFM extension as Markdown, without mounting any message or DOM. */
export const markdownSearchText = (source: string): string => {
  const parts: string[] = []
  const visit = (node: React.ReactNode): void => {
    React.Children.forEach(node, child => {
      if (typeof child === 'string' || typeof child === 'number') { parts.push(String(child)); return }
      if (!React.isValidElement<{ children?: React.ReactNode }>(child)) return
      // Image URLs, alt attributes and task-list controls are not rendered text leaves.
      if (child.type === 'img' || child.type === 'input') return
      const block = typeof child.type === 'string' && blocks[child.type] === true
      if (block) parts.push(' ')
      visit(child.props.children)
      if (block) parts.push(' ')
    })
  }
  visit(ReactMarkdown({ children: source, skipHtml: true, remarkPlugins }))
  return normalizeSearchText(parts.join(''))
}

interface TextPart { readonly text: string; readonly node?: Text }
interface Position { readonly node: Text; readonly offset: number }

/** Collapse rendered whitespace and case-fold per code point, preserving offsets even when folding expands. */
const normalized = (parts: readonly TextPart[]) => {
  let text = ''
  const starts: (Position | undefined)[] = [], ends: (Position | undefined)[] = []
  for (const part of parts) {
    let offset = 0
    for (const character of part.text) {
      const start = part.node === undefined ? undefined : { node: part.node, offset }
      offset += character.length
      const end = part.node === undefined ? undefined : { node: part.node, offset }
      // Fold final sigma too: matching must not depend on word position or inline-node boundaries.
      const folded = /\s/u.test(character) ? ' ' : character === 'ς' ? 'σ' : character.toLowerCase()
      if (folded === ' ' && (text === '' || text.endsWith(' '))) {
        if (text.endsWith(' ') && end !== undefined) ends[text.length - 1] = end
        continue
      }
      text += folded
      for (let index = 0; index < folded.length; index++) { starts.push(start); ends.push(end) }
    }
  }
  return { text: text.trimEnd(), starts, ends }
}
export const normalizeSearchText = (text: string): string => normalized([{ text }]).text
export const normalizeQuery = normalizeSearchText

/** Nonoverlapping occurrences, shared by the source index and DOM highlight mapping. */
export const searchOffsets = (text: string, needle: string): readonly number[] => {
  const offsets: number[] = []
  if (needle === '') return offsets
  for (let at = text.indexOf(needle); at !== -1; at = text.indexOf(needle, at + needle.length)) offsets.push(at)
  return offsets
}

const isChrome = (element: Element) => element.matches('[aria-hidden="true"], [hidden], [data-search-exclude], time, [data-call-meta], [data-testid="message-header"], [data-testid="markdown-code-language"], [data-testid="deferred-image"]') || (element.matches('button:not([data-search-content])') && element.querySelector('[data-call-title]') === null)

/** Search the same rendered text as the source projection, mapping each normalized occurrence back to text leaves. */
export const rangesIn = (root: Element, query: string): readonly Range[] => {
  const needle = normalizeQuery(query)
  if (needle === '') return []
  let parts: TextPart[] = []
  const groups: TextPart[][] = []
  const visit = (node: Node): void => {
    if (node.nodeType === 3) { parts.push({ text: node.textContent ?? '', node: node as Text }); return }
    if (node.nodeType !== 1) return
    const element = node as Element
    if (isChrome(element)) return
    if (element.matches('[data-testid="markdown-code-block"]')) {
      const code = element.querySelector('[data-testid="markdown-code"]')
      if (code !== null) visit(code)
      return
    }
    const block = blocks[element.tagName.toLowerCase()] === true
    if (block) parts.push({ text: ' ' })
    for (const child of element.childNodes) visit(child)
    if (block) parts.push({ text: ' ' })
  }
  // Mail names and action chrome are not part of its title/body search projection.
  const mail = root.matches('[data-testid="mail-message"]') ? root : root.querySelector('[data-testid="mail-message"]')
  if (mail !== null) {
    for (const content of mail.querySelectorAll('[data-search-mail-subject], [data-testid="markdown"]')) {
      parts.push({ text: ' ' })
      visit(content)
    }
  } else if (root.matches('[data-tool-status]') || root.querySelector('[data-tool-status]') !== null) {
    for (const label of root.querySelectorAll('[data-call-title]')) visit(label)
    groups.push(parts)
    parts = []
    for (const detail of root.querySelectorAll('[data-search-tool-summary], [data-search-tool-input], [data-search-tool-output]')) {
      parts.push({ text: ' ' })
      visit(detail)
    }
  } else visit(root)
  groups.push(parts)
  return groups.flatMap(group => {
    const { text, starts, ends } = normalized(group)
    return searchOffsets(text, needle).flatMap(offset => {
      const from = starts[offset], to = ends[offset + needle.length - 1]
      if (from === undefined || to === undefined) return []
      const range = root.ownerDocument.createRange()
      range.setStart(from.node, from.offset)
      range.setEnd(to.node, to.offset)
      return [range]
    })
  })
}
