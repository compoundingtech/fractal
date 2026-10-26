import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Hooks accept Fractal environment names',
  summary: 'Shell scripts and coding-agent hooks accept FRACTAL_* environment variables before their older ST_* names. Identity conflicts report the variable names and select neither identity. Explicit empty Fractal values mask older settings.',
  kind: 'fix',
} satisfies ChangelogEntrySource
