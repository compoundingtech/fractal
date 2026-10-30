import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ChangelogView } from './ChangelogView.tsx'
import type { ChangelogManifest } from './contract.ts'
import { initialSeen } from './seen.ts'

// As in the app's other Node render tests, only the compile-time CSS runtime is stubbed.
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

const manifest: ChangelogManifest = {
  schemaVersion: 1, product: 'fractal-web', build: { machineVersion: '0.1.0+local.123456789abc' },
  entries: [
    { id: '2026-10-11-context', date: '2026-10-11', visibility: 'public', title: 'Context usage', summary: 'Context usage is visible in the sidebar.', kind: 'feature' },
    { id: '2026-10-10-preview', date: '2026-10-10', visibility: 'private', title: 'Preview workflow', summary: 'Preview-only controls are available in this deployment.' },
    { id: '2026-10-09-tree', date: '2026-10-09', visibility: 'public', title: 'Tree alignment', summary: 'Nested rows now align consistently.', kind: 'fix' },
  ],
}
describe('changelog view', () => {
  it('first visit never claims a since-last-visit diff', () => {
    const html = renderToStaticMarkup(<ChangelogView state={{ _tag: 'Ready', manifest }} baseline={initialSeen} onRetry={() => undefined} />)
    expect(html).toContain('Recent changes')
    expect(html).not.toContain('New since you last looked')
    expect(html).toContain(manifest.build.machineVersion)
    expect(html).toContain('private')
  })
  it('places new changes before recent changes in one view', () => {
    const html = renderToStaticMarkup(<ChangelogView state={{ _tag: 'Ready', manifest }} baseline={{ visited: true, entryIds: ['2026-10-09-tree'] }} onRetry={() => undefined} />)
    expect(html.indexOf('New since you last looked')).toBeLessThan(html.indexOf('>Recent<'))
    expect(html.indexOf('Context usage')).toBeLessThan(html.indexOf('Preview workflow'))
    expect(html.indexOf('Preview workflow')).toBeLessThan(html.indexOf('Tree alignment'))
  })
  it('shows only recent after acknowledgement', () => {
    const html = renderToStaticMarkup(<ChangelogView state={{ _tag: 'Ready', manifest }} baseline={{ visited: true, entryIds: manifest.entries.map((entry) => entry.id) }} onRetry={() => undefined} />)
    expect(html).toContain('>Recent<')
    expect(html).not.toContain('New since you last looked')
  })
  it('shows a short canonical revision and labelled kinds without changing manifest identity', () => {
    const html = renderToStaticMarkup(<ChangelogView state={{ _tag: 'Ready', manifest }} onRetry={() => undefined} buildRevision="123456789abc" />)
    expect(html).toContain('>1234567</p>')
    expect(html).not.toContain(' title=')
    expect(html).not.toContain(`>${manifest.build.machineVersion}</p>`)
    expect(html).toContain('data-changelog-kind="feature"')
    expect(html).toContain('>Feature</span>')
    expect(html).toContain('>Fix</span>')
  })
  it('has loading, empty and failure states with recovery', () => {
    expect(renderToStaticMarkup(<ChangelogView state={{ _tag: 'Loading' }} onRetry={() => undefined} />)).toContain('Loading recent changes')
    expect(renderToStaticMarkup(<ChangelogView state={{ _tag: 'Ready', manifest: { ...manifest, entries: [] } }} onRetry={() => undefined} />)).toContain('No changes yet')
    const failed = renderToStaticMarkup(<ChangelogView state={{ _tag: 'Failed' }} onRetry={() => undefined} />)
    expect(failed).toContain('Retry')
    expect(failed).toContain('workspace is unaffected')
  })
})
