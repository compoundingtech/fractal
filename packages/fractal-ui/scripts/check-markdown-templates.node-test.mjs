import assert from 'node:assert/strict'
import { test } from 'node:test'
import { findViolations } from './check-markdown-templates.mjs'

test('a plain value spliced into a Markdown template is caught', () => {
  assert.deepEqual(findViolations('markdownBlock(`Mission ${mission.title} is ready`)'), ['input.ts:1: mission.title'])
  assert.deepEqual(findViolations("markdownInline('Run ' + command)"), ['input.ts:1: command'])
})

test('escaped, fenced and already-Markdown splices pass', () => {
  assert.deepEqual(findViolations('markdownBlock(`Mission ${inlineFromPlain(title)}:\\n\\n${markdownSource(summary)}\\n${markdownCodeFence(log)}`)'), [])
  assert.deepEqual(findViolations("markdownInline('Run ' + inlineFromPlain(command))"), [])
  assert.deepEqual(findViolations("markdownBlock('static text')"), [])
})

test('a reviewed exception needs its reason on the line above', () => {
  assert.deepEqual(findViolations('// markdown-template-reviewed: a number\nmarkdownBlock(`Answer ${index}`)'), [])
  assert.deepEqual(findViolations('// markdown-template-reviewed:\nmarkdownBlock(`Answer ${index}`)'), ['input.ts:2: index'])
})
