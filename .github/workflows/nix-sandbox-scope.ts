// TEMPORARY: retire when effect-utils offers an opt-in/opt-out for linuxNixSandboxSettings
// (genie/ci-workflow/shared.ts at effect-utils 8cb2135bd4a852b99ca3f677c0815d0268766ff5 injects
// `sandbox = true` and `sandbox-fallback = false` into every installer extra-conf).
//
// `sandbox-fallback = false` turns Nix's normal fallback into a hard failure on a runner whose kernel or
// AppArmor policy forbids unprivileged user namespaces. Only the jobs that enable user namespaces first
// (the Buck TypeScript lanes, which run `sysctl kernel.apparmor_restrict_unprivileged_userns=0`) can keep it.
const strictSandboxLines = new Set([
  "${{ runner.os == 'Linux' && 'sandbox = true' || '' }}",
  "${{ runner.os == 'Linux' && 'sandbox-fallback = false' || '' }}",
])

const enablesUserNamespaces = (job: { steps?: readonly any[] }) =>
  (job.steps ?? []).some(step => typeof step?.run === 'string' && step.run.includes('kernel.apparmor_restrict_unprivileged_userns=0'))

const stripStrictSandbox = (step: any) => {
  if (step?.with === undefined) return step
  let changed = false
  const withArgs = { ...step.with }
  for (const key of ['extra-conf', 'extra_nix_config'] as const) {
    const value = withArgs[key]
    if (typeof value !== 'string') continue
    const lines = value.split('\n')
    const kept = lines.filter(line => !strictSandboxLines.has(line.trim()))
    if (kept.length === lines.length) continue
    withArgs[key] = kept.join('\n')
    changed = true
  }
  return changed ? { ...step, with: withArgs } : step
}

/** Keeps the strict Linux Nix sandbox settings only in jobs that enable user namespaces first. */
export const scopeLinuxNixSandbox = <T extends { jobs: Record<string, any> }>(workflow: T): T => ({
  ...workflow,
  jobs: Object.fromEntries(Object.entries(workflow.jobs).map(([name, job]) =>
    [name, enablesUserNamespaces(job) || job.steps === undefined ? job : { ...job, steps: job.steps.map(stripStrictSandbox) }])),
}) as T
