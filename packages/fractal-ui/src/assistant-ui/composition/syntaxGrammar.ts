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

// Fixed allowlist: authored languages never select a module URL.
for (const grammar of [typescript, tsx, javascript, json, bash, diff, rust, nix, python, yaml, markdown, css]) refractor.register(grammar)

export const highlightSource = (code: string, language: string) => refractor.highlight(code, language).children
