import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Input-to-paint diagnostics',
  summary: 'When browser trace export is configured, inputs with owned control, field, transcript-tail or overlay commits report paint latency and whether they painted, were superseded, or ended before paint. No input content is exported; browser telemetry uses the distinct fractal-web service identity.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
