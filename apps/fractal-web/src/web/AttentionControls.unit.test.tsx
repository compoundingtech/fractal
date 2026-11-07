import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { SidebarAgent, SidebarFolder } from '../folders/sidebarContract.ts'
import { FolderAttentionSignal } from './AttentionControls.tsx'

// Match the app's node-rendering lane: exercise semantic DOM without the CSS compiler.
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const agent = (subject: string): SidebarAgent => ({ _tag: 'Agent', id: subject, subject, label: subject })
const folder = (subjects: readonly string[]): SidebarFolder => ({ _tag: 'Folder', id: 'folder', label: 'Folder', collapsed: false, children: subjects.map(agent) })
const signal = (subjects: readonly string[], refs: readonly string[]) =>
  renderToStaticMarkup(createElement(FolderAttentionSignal, { folder: folder(subjects), refs: new Set(refs) }))

describe('folder attention signal', () => {
  it('agrees in number with the count it shows', () => {
    expect(signal(['a', 'b'], ['a'])).toContain('>1 needs you</span>')
    expect(signal(['a', 'b'], ['a', 'b'])).toContain('>2 need you</span>')
  })

  it('stays silent when nothing in the folder needs you', () => {
    expect(signal(['a'], [])).toBe('')
  })
})
