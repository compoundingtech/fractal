import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { RootContent as SyntaxNode } from 'hast'
import { refractor } from 'refractor/core'
import typescript from 'refractor/typescript'
import tsx from 'refractor/tsx'
import javascript from 'refractor/javascript'
import json from 'refractor/json'
import bash from 'refractor/bash'
import diff from 'refractor/diff'
import rust from 'refractor/rust'
import nix from 'refractor/nix'
import python from 'refractor/python'
import yaml from 'refractor/yaml'
import markdown from 'refractor/markdown'
import css from 'refractor/css'
import { textVars as textColor, statusVars as status, typeVars as t } from '../composition-tokens.stylex'

// Fixed allowlist: authored languages never select a module URL.
for (const grammar of [typescript, tsx, javascript, json, bash, diff, rust, nix, python, yaml, markdown, css]) refractor.register(grammar)

function SyntaxToken({ node }: { node: SyntaxNode }): React.ReactNode {
  if (node.type === 'text') return node.value
  if (node.type !== 'element') return null
  const classes = Array.isArray(node.properties.className) ? node.properties.className.map(String) : []
  return <span data-syntax-token={classes.filter(value => value !== 'token').join(' ')} {...stylex.props(
    classes.some(value => ['comment', 'prolog', 'doctype', 'cdata'].includes(value)) && styles.comment,
    classes.some(value => ['keyword', 'atrule', 'tag', 'selector'].includes(value)) && styles.keyword,
    classes.some(value => ['string', 'char', 'attr-value', 'inserted'].includes(value)) && styles.string,
    classes.some(value => ['number', 'boolean', 'constant', 'symbol'].includes(value)) && styles.literal,
    classes.some(value => ['function', 'class-name', 'builtin'].includes(value)) && styles.function,
    classes.includes('deleted') && styles.deleted,
  )}>{node.children.map((child, index) => <SyntaxToken key={index} node={child} />)}</span>
}

export default function SyntaxSource({ code, language }: { code: string; language: string }) {
  const nodes = React.useMemo(() => refractor.highlight(code, language).children, [code, language])
  return <>{nodes.map((node, index) => <SyntaxToken key={index} node={node} />)}</>
}

const styles = stylex.create({
  comment: { color: textColor.fgMuted, fontStyle: 'italic' },
  keyword: { color: status.runningFg },
  string: { color: textColor.fgMuted },
  literal: { color: `var(--fractal-diff-literal, ${status.attention})` },
  function: { color: textColor.fg, fontWeight: t.weightMedium },
  deleted: { color: status.dangerFg },
})
