/** Capture this workflow's actual dependency names for runner-wait alerts.
 * The hosted observer uses metadata only and never executes queued PR source.
 */
export const withQueueWatch = <T extends { jobs: Record<string, any> }>(workflow: T): T => {
  const dependencies = Object.fromEntries(Object.entries(workflow.jobs).map(([id, job]) => {
    const needs = job.needs == null ? [] : typeof job.needs === 'string' ? [job.needs] : job.needs
    return [job.name ?? id, needs.map((dependency: string) => workflow.jobs[dependency].name ?? dependency)]
  }))
  const observer = workflow.jobs['namespace-capacity']
  return { ...workflow, jobs: { ...workflow.jobs, 'namespace-capacity': {
    ...observer,
    steps: observer.steps.map((step: any) => step.name === 'Cancel superseded merge groups'
      ? { ...step, env: { ...step.env, QUEUE_DEPENDENCIES: JSON.stringify(dependencies) } } : step),
  } } } as T
}
