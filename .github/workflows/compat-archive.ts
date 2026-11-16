import { auditCaches } from './cache-audit.ts'

type Kind = 'fleet' | 'messaging'
type Step = { name?: string; uses?: string; [key: string]: unknown }
type Job = { steps: readonly Step[]; needs?: string | readonly string[]; [key: string]: unknown }
const linux = 'x86_64-unknown-linux-gnu'
const darwin = 'aarch64-apple-darwin'
const identity = (kind: Kind, target: string, id: string) => ({
  name: `Identify the pinned ${kind} archive`, id, shell: 'bash',
  run: `python3 scripts/ci-compat-archive identity ${kind} ${target}`,
})
const cache = (id: string) => ({
  path: `\${{ steps['${id}'].outputs['cache-path'] }}`,
  key: `\${{ steps['${id}'].outputs.key }}`,
})

/** Prepare once per native target. Runtime identities keep baseline-release pin updates self-contained. */
export const withCompatArchives = <T extends { jobs: Record<string, Job> }>(workflow: T): T => {
  const jobs: Record<string, Job> = { ...workflow.jobs }
  const targets = new Map<string, Set<Kind>>()
  for (const [id, job] of Object.entries(jobs)) {
    // Evidence gates inspect receipts only; they never execute compatibility tests.
    if (job.steps.some((step) => step.id === 'gate'
      && typeof step.run === 'string' && step.run.includes('scripts/ci-evidence-gate'))) continue
    // Shared historical setup covers test-build, archive consumers and cache warmers.
    const messaging = job.steps.some((step) => step.name === 'Prepare the historical messaging channel')
      || id === 'macos-ci'
    const fleet = id.endsWith('fleet-compat')
    if (!messaging && !fleet) continue
    if (messaging && fleet) throw new Error(`Compatibility consumer ${id} must select one baseline kind`)
    const kind: Kind = fleet ? 'fleet' : 'messaging'
    const target = id === 'macos-ci' ? darwin : linux
    const producer = `compat-archives-${target}`
    const kinds = targets.get(target) ?? new Set<Kind>()
    kinds.add(kind)
    targets.set(target, kinds)
    const needs = job.needs == null ? [] : typeof job.needs === 'string' ? [job.needs] : job.needs
    const checkout = job.steps.findIndex((step) => step.uses?.startsWith('actions/checkout@'))
    if (checkout < 0) throw new Error(`Compatibility consumer ${id} needs a checkout`)
    // Keep the checkout/full-history fetch pair adjacent for source provenance.
    const checkoutEnd = checkout + (job.steps[checkout + 1]?.name === 'Fetch the whole history without evidence tags' ? 2 : 1)
    jobs[id] = { ...job, needs: [...needs, producer], steps: [
      ...job.steps.slice(0, checkoutEnd),
      { name: 'Require successful archive preparation', shell: 'bash',
        env: { PRODUCER_RESULT: `\${{ needs['${producer}'].result }}` },
        run: '[ "$PRODUCER_RESULT" = success ] || { echo "::error::compatibility archive producer failed or was skipped"; exit 1; }' },
      identity(kind, target, 'compat-identity'),
      { name: 'Require an explicit prepared archive name', shell: 'bash',
        env: { COMPAT_ARCHIVE_NAME: `\${{ needs['${producer}'].outputs['artifact-${kind}'] }}` },
        run: '[ -n "$COMPAT_ARCHIVE_NAME" ] || { echo "::error::compatibility archive producer returned an empty artifact name; refusing to download all run artifacts"; exit 1; }' },
      { name: 'Download the prepared compatibility archive', id: 'compat-archive', shell: 'bash',
        env: { GH_TOKEN: '${{ github.token }}',
          COMPAT_ARCHIVE_NAME: `\${{ needs['${producer}'].outputs['artifact-${kind}'] }}`,
          CI_TEST_ARCHIVE_ARTIFACT_ID: `\${{ needs['${producer}'].outputs['artifact-id-${kind}'] }}` },
        run: `python3 scripts/ci-compat-archive download ${kind} ${target} "\${{ steps['compat-identity'].outputs.path }}"` },
      ...job.steps.slice(checkoutEnd),
    ] }
  }
  for (const [target, kinds] of targets) {
    jobs[`compat-archives-${target}`] = {
      name: `Prepare verified compatibility archives (${target})`,
      'runs-on': 'ubuntu-latest', 'timeout-minutes': 15,
      permissions: { contents: 'read' },
      outputs: Object.fromEntries([...kinds].flatMap((kind, index) => [
        [`artifact-${kind}`, `\${{ steps['identity-${index}'].outputs['artifact-name'] }}`],
        [`artifact-id-${kind}`, index === 0 ? '${{ steps.upload-0.outputs.artifact-id }}'
          : '${{ steps.upload-1.outputs.artifact-id || steps.upload-0.outputs.artifact-id }}'],
      ])),
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        ...[...kinds].map((kind, index) => identity(kind, target, `identity-${index}`)),
        ...[...kinds].flatMap((kind, index) => {
          // Two kinds with identical content share one restore, verification, download and save.
          const condition = index === 0 ? {} : { if: "steps.identity-1.outputs.sha256 != steps.identity-0.outputs.sha256" }
          return [
            { name: `Restore ${kind} archive`, id: `archive-${index}`, ...condition,
              uses: 'actions/cache/restore@v4', with: cache(`identity-${index}`) },
            { name: `Verify and prepare ${kind} archive`, shell: 'bash', ...condition,
              env: { GH_TOKEN: '${{ github.token }}',
                COMPAT_ARCHIVE_CACHE_HIT: `\${{ steps.archive-${index}.outputs.cache-hit }}` },
              run: `python3 scripts/ci-compat-archive prepare ${kind} ${target} "\${{ steps['identity-${index}'].outputs.path }}"` },
            { name: `Save verified ${kind} archive`,
              if: `${index === 0 ? '' : "steps.identity-1.outputs.sha256 != steps.identity-0.outputs.sha256 && "}steps.archive-${index}.outputs.cache-hit != 'true'`,
              uses: 'actions/cache/save@v4', with: cache(`identity-${index}`) },
            { name: `Share verified ${kind} archive`, id: `upload-${index}`, ...condition,
              uses: 'actions/upload-artifact@v4', with: {
                name: `\${{ steps['identity-${index}'].outputs['artifact-name'] }}`,
                path: `\${{ steps['identity-${index}'].outputs.path }}`,
                'if-no-files-found': 'error', 'retention-days': 7, 'compression-level': 0,
              } },
          ]
        }),
      ],
    }
    const producerId = `compat-archives-${target}`
    jobs[producerId] = auditCaches({ jobs: { [producerId]: jobs[producerId] } }).jobs[producerId]
  }
  return { ...workflow, jobs }
}
