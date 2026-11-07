import { describe, expect, it } from 'vitest'

import { conversationFixture, filesFixture, fixtureNow, pullRequestFixture, pullRequestRead, stalePullRequestRead } from '../fixtures.ts'
import { agoWord, conversationTimeline, countOf, diffLinesOf, filesView, labelColor, pullRequestFilesUrl, staleWord, tabLabel } from './detail-model.ts'

describe('diffLinesOf', () => {
  it('drops only the final newline tail, keeping a lone added blank line', () => {
    expect(diffLinesOf('@@ -0,0 +1,4 @@\n+pub struct Scheduler {\n+    beds: Vec<Bed>,\n+}\n+')).toEqual(['@@ -0,0 +1,4 @@', '+pub struct Scheduler {', '+    beds: Vec<Bed>,', '+}', '+'])
    expect(diffLinesOf('@@ -1 +1 @@\n-a\n+b\n')).toEqual(['@@ -1 +1 @@', '-a', '+b'])
    expect(diffLinesOf(' a\n\n b\n')).toEqual([' a', '', ' b'])
  })
})

describe('filesView', () => {
  it('maps patched files to DiffPanel files and keeps patchless ones for links', () => {
    const view = filesView(filesFixture)
    expect(view.diffs.map(file => [file.path, file.added, file.removed])).toEqual([['src/scheduler.rs', 180, 0], ['src/beds.rs', 34, 37]])
    expect(view.diffs[1]!.diff[0]).toBe('@@ -10,3 +10,3 @@ impl Bed {')
    expect(view.withoutPatch.map(file => file.path)).toEqual(['assets/beds.png'])
    expect(view.partial).toBe(false)
  })
  it('is partial when truncated, past GitHub\'s 3000-file list, or short of the total', () => {
    expect(filesView({ ...filesFixture, truncated: true }).partial).toBe(true)
    expect(filesView({ ...filesFixture, total: 3001 }).partial).toBe(true)
    expect(filesView({ ...filesFixture, total: 4 }).partial).toBe(true)
  })
  it('links the PR files page', () => {
    expect(pullRequestFilesUrl(pullRequestFixture)).toBe('https://github.com/acme/garden/pull/7/files')
  })
})

describe('conversationTimeline', () => {
  it('orders reviews, threads and comments oldest first', () => {
    const timeline = conversationTimeline(conversationFixture)
    const at = timeline.map(entry => entry.at)
    expect(at).toEqual([...at].sort())
    expect(timeline.map(entry => entry.kind)).toContain('thread')
    expect(timeline.length).toBe(conversationFixture.reviews.length + conversationFixture.review_threads.length + conversationFixture.comments.length)
  })
  it('puts undated reviews last', () => {
    const [first] = conversationFixture.reviews
    const timeline = conversationTimeline({ reviews: [{ ...first!, submitted_at: null }], review_threads: [], comments: conversationFixture.comments })
    expect(timeline.at(-1)?.kind).toBe('review')
  })
})

describe('words', () => {
  it('counts a found read, else falls back', () => {
    expect(countOf(pullRequestRead, () => 3)).toBe(3)
    expect(countOf(undefined, () => 3, 7)).toBe(7)
    expect(countOf({ status: 'gone' }, () => 3)).toBeUndefined()
    expect(tabLabel('Files', 3)).toBe('Files 3')
    expect(tabLabel('Files', undefined)).toBe('Files')
  })
  it('says how stale a read is', () => {
    expect(staleWord(stalePullRequestRead, fixtureNow)).toBe('stale · fetched 5m ago')
    expect(staleWord(pullRequestRead, fixtureNow)).toBeUndefined()
    expect(agoWord('2026-11-07T06:44:30Z', fixtureNow)).toBe('just now')
    expect(agoWord('2026-11-04T06:45:00Z', fixtureNow)).toBe('3d ago')
  })
  it('accepts only six-digit label colours', () => {
    expect(labelColor('a2eeef')).toBe('#a2eeef')
    expect(labelColor('red;background:url(x)')).toBeUndefined()
  })
})
