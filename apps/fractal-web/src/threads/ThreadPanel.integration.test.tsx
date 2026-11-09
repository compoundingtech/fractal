// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  props: () => ({}),
}))

import { ThreadPanel } from './ThreadPanel.tsx'

it('collapses the entire embedded conversation and keeps lifecycle state authoritative', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const change = vi.fn(async () => undefined)
  const render = (state: 'open' | 'closed') => act(async () => {
    root.render(<ThreadPanel thread={{ title: 'PR #42', title_revision: 'event/one', state }} messageCount={3} onChangeState={change}>
      <textarea aria-label="Thread message" />
    </ThreadPanel>)
  })
  const press = async (name: string) => {
    const button = Array.from(host.querySelectorAll('button')).find(button => button.textContent?.includes(name))
    expect(button).toBeDefined()
    await act(async () => button?.click())
  }
  try {
    await render('open')
    const header = host.querySelector('button')
    expect(header?.getAttribute('aria-expanded')).toBe('false')
    expect(header?.textContent).toContain('3 messages')
    expect(host.querySelector('textarea')).toBeNull()

    await press('PR #42')
    expect(header?.getAttribute('aria-expanded')).toBe('true')
    expect(host.querySelector('textarea')).not.toBeNull()
    await press('Close thread')
    expect(change).toHaveBeenCalledWith('closed')
    expect(host.textContent).toContain('Close thread')

    await render('closed')
    expect(host.textContent).toContain('Reopen thread')
    await press('Reopen thread')
    expect(change).toHaveBeenLastCalledWith('open')
    await press('PR #42')
    expect(header?.textContent).toContain('3 messages · Closed')
    expect(host.querySelector('textarea')).toBeNull()
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it('keeps the title revision that the person began editing after a concurrent rename', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const rename = vi.fn(async () => 'The title changed. Review the latest title and try again.')
  const render = (title: string, revision: string) => act(async () => {
    root.render(<ThreadPanel thread={{ title, title_revision: revision, state: 'open' }} embedded={false} onRename={rename}>
      Thread contents
    </ThreadPanel>)
  })
  const press = async (label: string) => {
    const button = Array.from(host.querySelectorAll('button')).find(button => button.textContent === label)
    expect(button).toBeDefined()
    await act(async () => button?.click())
  }
  try {
    await render('PR #42', 'event/old')
    await press('Rename')
    await render('Worker check', 'event/new')
    expect(host.querySelector('h2')?.textContent).toBe('Worker check')
    expect(host.querySelector('input')?.value).toBe('PR #42')
    await press('Save')
    expect(rename).toHaveBeenCalledWith('PR #42', 'event/old')
    expect(host.querySelector('[role=alert]')?.textContent).toContain('The title changed')
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
